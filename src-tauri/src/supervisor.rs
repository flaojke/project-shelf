use crate::{
    adapters::{
        android::{self, AndroidConfig},
        docker, powershell,
        process::{self, Probe},
    },
    diagnostics,
    models::{
        DiagnosticState, LogEntry, Phase, Profile, ProjectDiagnostics, ProjectKind, ProjectStatus,
        Settings,
    },
    runtime::{self, ManagedProcess},
    store::Store,
    validate,
};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::sync::{Mutex, RwLock};

enum Owned {
    Vite {
        process: ManagedProcess,
    },
    Docker {
        ids: Vec<String>,
    },
    Android {
        serial: String,
        app_pid: Option<u32>,
        emulator: Option<ManagedProcess>,
        avd: Option<String>,
    },
}
#[derive(Default)]
struct Slot {
    resource: Option<Owned>,
}

#[derive(Debug, PartialEq, Eq)]
enum StartDecision {
    Reuse,
    Spawn,
}
fn start_decision(probe: &Probe, owned: bool) -> Result<StartDecision, String> {
    match probe {
        Probe::Ready { .. } => Ok(StartDecision::Reuse),
        Probe::Conflict(message) => Err(message.clone()),
        Probe::Missing if owned => Err(
            "Проект уже запущен этим сеансом, но ещё не готов. Второй экземпляр не создаётся."
                .into(),
        ),
        Probe::Missing => Ok(StartDecision::Spawn),
    }
}
fn stop_permission(kind: &ProjectKind, owned: bool) -> Result<(), String> {
    if *kind == ProjectKind::Powershell {
        return Err("У PowerShell нет безопасной команды завершения. Принудительная остановка запрещена из-за файловых записей; сервер оставлен работающим.".into());
    }
    if !owned {
        return Err(
            "Проект не принадлежит текущему сеансу Project Shelf. Внешняя остановка запрещена."
                .into(),
        );
    }
    Ok(())
}
fn readiness_expired(now: tokio::time::Instant, deadline: tokio::time::Instant) -> bool {
    now >= deadline
}
fn wrapper_exit_result(exit: Option<(bool, Option<i32>)>) -> Result<bool, String> {
    match exit {
        None=>Ok(false),
        Some((true,_))=>Ok(true),
        Some((false,code))=>Err(format!("Штатный PowerShell launcher завершился с ошибкой (код {}). Сервер не подтвердил готовность. Проверьте файлы проекта и возможность запуска PowerShell; принудительная остановка не выполнялась.",code.map(|v|v.to_string()).unwrap_or_else(||"неизвестен".into()))),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AndroidTarget {
    serial: String,
    new_emulator: bool,
}
fn android_target(
    explicit: Option<&str>,
    matching_avd: Option<&str>,
    devices: &[android::AndroidDevice],
) -> Result<AndroidTarget, String> {
    if let Some(serial) = matching_avd {
        return Ok(AndroidTarget {
            serial: serial.into(),
            new_emulator: false,
        });
    }
    if let Some(serial) = explicit {
        android::validate_serial(serial)?;
        return Ok(AndroidTarget {
            serial: serial.into(),
            new_emulator: !devices.iter().any(|d| d.serial == serial),
        });
    }
    if devices.iter().any(|d| d.serial == "emulator-5554") {
        return Err("emulator-5554 занят другим устройством. Автоматический запуск выбранного AVD невозможен на этом порту; выберите serial явно.".into());
    }
    Ok(AndroidTarget {
        serial: "emulator-5554".into(),
        new_emulator: true,
    })
}
fn emulator_port(serial: &str) -> Result<u16, String> {
    serial
        .strip_prefix("emulator-")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "Не подтверждён порт выбранного эмулятора.".into())
}

pub struct Supervisor {
    profiles: RwLock<Vec<Profile>>,
    settings: RwLock<Settings>,
    store: Store,
    write_lock: Mutex<()>,
    slots: HashMap<String, Mutex<Slot>>,
    statuses: RwLock<HashMap<String, ProjectStatus>>,
    logs: StdMutex<HashMap<String, VecDeque<LogEntry>>>,
    client: reqwest::Client,
    pub exiting: AtomicBool,
    quitting: AtomicBool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuitSummary {
    pub managed: Vec<ProjectStatus>,
    pub retained: Vec<String>,
    pub failed: Vec<String>,
}

impl Supervisor {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let store = Store::new(root)?;
        let profiles = store.load_profiles()?;
        let settings = store.load_settings()?;
        let mut slots = HashMap::new();
        let mut statuses = HashMap::new();
        for profile in &profiles {
            let validation = validate::profile_validation(profile);
            slots.insert(profile.id.clone(), Mutex::new(Slot::default()));
            statuses.insert(
                profile.id.clone(),
                ProjectStatus::new(
                    &profile.id,
                    if validation.complete {
                        Phase::Stopped
                    } else {
                        Phase::Setup
                    },
                    false,
                    if validation.complete {
                        "Готов к запуску.".to_string()
                    } else {
                        validation.issues.join(" ")
                    },
                ),
            );
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| "Не удалось создать HTTP client.")?;
        Ok(Self {
            profiles: RwLock::new(profiles),
            settings: RwLock::new(settings),
            store,
            write_lock: Mutex::new(()),
            slots,
            statuses: RwLock::new(statuses),
            logs: StdMutex::new(HashMap::new()),
            client,
            exiting: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
        })
    }
    pub async fn profiles(&self) -> Vec<Profile> {
        self.profiles.read().await.clone()
    }
    pub async fn settings(&self) -> Settings {
        self.settings.read().await.clone()
    }
    pub async fn save_settings(&self, settings: Settings) -> Result<Settings, String> {
        validate::settings(&settings)?;
        let _guard = self.write_lock.lock().await;
        self.store.save_settings(&settings)?;
        *self.settings.write().await = settings.clone();
        Ok(settings)
    }
    pub async fn save_profile(&self, profile: Profile) -> Result<Profile, String> {
        validate::profile(&profile)?;
        let operation = self.slot(&profile.id)?.lock().await;
        let _write = self.write_lock.lock().await;
        let mut profiles = self.profiles.write().await;
        let current = profiles
            .iter()
            .find(|p| p.id == profile.id)
            .ok_or("Профиль не зарегистрирован.")?;
        validate::editing_allowed(current, &profile, operation.resource.is_some())?;
        let runtime_changed = current.path != profile.path
            || current.url != profile.url
            || current.config != profile.config;
        let mut next = profiles.clone();
        let index = next
            .iter()
            .position(|p| p.id == profile.id)
            .ok_or("Профиль не найден.")?;
        next[index] = profile.clone();
        self.store.save_profiles(&next)?;
        *profiles = next;
        if runtime_changed {
            let validation = validate::profile_validation(&profile);
            self.phase(
                &profile.id,
                if validation.complete {
                    Phase::Stopped
                } else {
                    Phase::Setup
                },
                false,
                &if validation.complete {
                    "Конфигурация сохранена.".into()
                } else {
                    validation.issues.join(" ")
                },
            )
            .await;
        }
        Ok(profile)
    }
    pub async fn profile(&self, id: &str) -> Result<Profile, String> {
        self.profiles
            .read()
            .await
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "Профиль не зарегистрирован.".into())
    }
    fn slot(&self, id: &str) -> Result<&Mutex<Slot>, String> {
        self.slots
            .get(id)
            .ok_or_else(|| "Неизвестный проект.".into())
    }
    async fn live(&self) -> Result<(), String> {
        if self.settings.read().await.live_actions_enabled {
            Ok(())
        } else {
            Err("Реальные действия отключены. Разрешите их в настройках; система ничего не запустила.".into())
        }
    }
    fn accept_action(&self) -> Result<(), String> {
        if self.quitting.load(Ordering::SeqCst) {
            return Err("Идёт выход из Project Shelf. Новые действия не запускаются.".into());
        }
        Ok(())
    }
    fn log(&self, id: &str, level: &str, message: &str) {
        // Only structured events reach this function. Never forward command
        // stdout, stderr, environment variables or container inspect payloads.
        let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        let entries = logs.entry(id.into()).or_default();
        entries.push_back(LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.into(),
            message: message
                .chars()
                .filter(|c| !c.is_control())
                .take(600)
                .collect(),
        });
        while entries.len() > 300 {
            entries.pop_front();
        }
    }
    pub async fn logs(&self, id: &str) -> Result<Vec<LogEntry>, String> {
        self.profile(id).await?;
        Ok(self
            .logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default())
    }
    async fn status(&self, id: &str) -> ProjectStatus {
        self.statuses
            .read()
            .await
            .get(id)
            .cloned()
            .unwrap_or_else(|| {
                ProjectStatus::new(id, Phase::Unavailable, false, "Неизвестный проект.")
            })
    }
    async fn set(&self, status: ProjectStatus) {
        self.statuses
            .write()
            .await
            .insert(status.id.clone(), status);
    }
    async fn phase(&self, id: &str, phase: Phase, owned: bool, message: &str) {
        let old = self.status(id).await;
        let mut next = ProjectStatus::new(id, phase, owned, message);
        if owned {
            next.pid = old.pid;
            next.serial = old.serial;
            next.started_at = old.started_at;
        }
        self.set(next).await;
    }

    pub async fn statuses(&self) -> Vec<ProjectStatus> {
        for profile in self.profiles().await {
            if self.quitting.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(mut slot) = self.slots[&profile.id].try_lock() {
                let current = self
                    .profile(&profile.id)
                    .await
                    .expect("fixed registered slot");
                if let Err(message) = self.refresh(&current, &mut slot).await {
                    self.phase(
                        &profile.id,
                        Phase::Unavailable,
                        slot.resource.is_some(),
                        &message,
                    )
                    .await;
                }
            }
        }
        let statuses = self.statuses.read().await;
        validate::ALLOWED_IDS
            .iter()
            .filter_map(|id| statuses.get(*id).cloned())
            .collect()
    }

    async fn http_probe(&self, profile: &Profile) -> Result<Probe, String> {
        validate::require_complete(profile)?;
        match profile.kind {
            ProjectKind::Process => {
                Ok(process::probe(&self.client, profile, &process::validate(profile)?).await)
            }
            ProjectKind::Powershell => {
                Ok(powershell::probe(&self.client, profile, &powershell::validate(profile)?).await)
            }
            ProjectKind::Docker => {
                let config = docker::validate(profile)?;
                let base = validate::loopback_url(profile.url.as_deref().ok_or("URL не задан.")?)?;
                if !process::port_open(base.port_or_known_default().ok_or("Порт не задан.")?).await
                {
                    return Ok(Probe::Missing);
                }
                let health = base
                    .join("health")
                    .map_err(|_| "Некорректный health URL.")?;
                match process::response(&self.client, health.as_str()).await {
                    Ok((200, text)) if serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("message").and_then(|v| v.as_str()).map(str::to_string)).as_deref() == Some(config.health_message.as_str()) => Ok(Probe::Ready { pid: None }),
                    _ => Ok(Probe::Conflict("Порт занят, но /health не подтверждает настроенный Docker-сервис. Чужой сервис не останавливается.".into())),
                }
            }
            ProjectKind::Android => Err("Android не использует HTTP readiness.".into()),
        }
    }

    async fn refresh(&self, profile: &Profile, slot: &mut Slot) -> Result<(), String> {
        let validation = validate::profile_validation(profile);
        if !validation.complete {
            self.phase(
                &profile.id,
                Phase::Setup,
                false,
                &validation.issues.join(" "),
            )
            .await;
            return Ok(());
        }
        if let Some(Owned::Vite { process }) = &mut slot.resource {
            if process.exited()? {
                slot.resource = None;
                self.phase(
                    &profile.id,
                    Phase::Stopped,
                    false,
                    "Собственный процесс завершился.",
                )
                .await;
            }
        }
        if profile.kind == ProjectKind::Android {
            return self.refresh_android(profile, slot).await;
        }
        let probe = self.http_probe(profile).await?;
        match probe {
            Probe::Ready { pid } => {
                if profile.kind == ProjectKind::Docker {
                    if !self.settings.read().await.live_actions_enabled {
                        self.phase(&profile.id,Phase::External,false,"Docker отвечает; Docker identity не проверена, реальные действия отключены.").await;
                        return Ok(());
                    }
                    let identity = docker::inspect(&docker::validate(profile)?).await?;
                    if !identity.running {
                        return Err("HTTP отвечает, но нужный Docker service не запущен. Возможен конфликт порта.".into());
                    }
                    if let Some(Owned::Docker { ids }) = &slot.resource {
                        if *ids != identity.container_ids {
                            slot.resource = None;
                        }
                    }
                }
                let owned = slot.resource.is_some();
                let old = self.status(&profile.id).await;
                let mut status = ProjectStatus::new(
                    &profile.id,
                    if owned { Phase::Ready } else { Phase::External },
                    owned,
                    if owned {
                        "Готов к работе."
                    } else {
                        "Уже работает вне текущего сеанса Project Shelf."
                    },
                );
                status.pid = pid.or(old.pid.filter(|_| owned));
                status.started_at = old.started_at.filter(|_| owned);
                self.set(status).await;
            }
            Probe::Missing => {
                if slot.resource.is_some() {
                    self.phase(&profile.id,Phase::Error,true,"Собственный ресурс запущен, но HTTP не готов. Можно остановить его или повторить проверку.").await;
                } else if profile.kind != ProjectKind::Docker
                    && !std::path::Path::new(&profile.path).is_dir()
                {
                    self.phase(
                        &profile.id,
                        Phase::Unavailable,
                        false,
                        "Каталог проекта не найден.",
                    )
                    .await;
                } else {
                    self.phase(&profile.id, Phase::Stopped, false, "Сейчас не запущен.")
                        .await;
                }
            }
            Probe::Conflict(message) => {
                self.phase(&profile.id, Phase::Error, slot.resource.is_some(), &message)
                    .await
            }
        }
        Ok(())
    }

    async fn refresh_android(&self, profile: &Profile, slot: &mut Slot) -> Result<(), String> {
        if !self.settings.read().await.live_actions_enabled {
            return Ok(());
        }
        // Background status reads use the existing daemon's raw read-only protocol.
        // Only an explicit user action may invoke an ADB client or resolve/start an AVD.
        let mut diagnostic_profile = profile.clone();
        let mut owned = false;
        let mut owned_serial = None;
        if let Some(Owned::Android {
            serial,
            emulator,
            app_pid,
            ..
        }) = &mut slot.resource
        {
            if let Some(process) = emulator {
                if !process.exited()? && process.owns_listener(emulator_port(serial)?)? {
                    owned = true;
                    owned_serial = Some(serial.clone());
                    diagnostic_profile.config["serial"] = serde_json::json!(serial);
                } else {
                    *app_pid = None;
                }
            }
        }
        if !owned {
            slot.resource = None;
        }
        let observation = android::diagnose(&diagnostic_profile).await;
        if let Some(Owned::Android { app_pid, .. }) = &mut slot.resource {
            // An unavailable measurement is not evidence that the owned app changed.
            if let Some(actual) = observation.app_pid {
                if app_pid.is_some_and(|expected| expected != actual) {
                    *app_pid = None;
                }
            }
        }
        let old = self.status(&profile.id).await;
        let phase = if observation.boot_completed == Some(true) && observation.app_pid.is_some() {
            if owned {
                Phase::Ready
            } else {
                Phase::External
            }
        } else if owned && observation.app_installed == Some(false) {
            Phase::Error
        } else {
            // No daemon/serial/AVD identity or legacy-shell absence is unknown, never "stopped".
            Phase::Unavailable
        };
        let mut status = ProjectStatus::new(&profile.id, phase, owned, observation.process_message);
        status.serial =
            observation
                .serial
                .or(owned_serial)
                .or_else(|| if owned { old.serial } else { None });
        status.pid = observation.app_pid;
        status.started_at = old.started_at.filter(|_| owned);
        self.set(status).await;
        Ok(())
    }

    async fn existing_serial(
        &self,
        config: &AndroidConfig,
        devices: &[android::AndroidDevice],
    ) -> Result<Option<String>, String> {
        if let Some(serial) = config.serial.as_ref().filter(|s| !s.is_empty()) {
            return Ok(devices
                .iter()
                .find(|d| d.serial == *serial)
                .map(|d| d.serial.clone()));
        }
        let Some(avd) = &config.avd else {
            return Ok(None);
        };
        let mut matches = Vec::new();
        for device in devices
            .iter()
            .filter(|d| d.serial.starts_with("emulator-") && d.state == "device")
        {
            if android::avd_name(config, &device.serial)
                .await
                .ok()
                .as_deref()
                == Some(avd)
            {
                matches.push(device.serial.clone());
            }
        }
        if matches.len() > 1 {
            return Err(
                "Найдено несколько совпадающих AVD. Выберите точный serial в списке устройств."
                    .into(),
            );
        }
        Ok(matches.pop())
    }

    pub async fn start(&self, id: &str) -> Result<ProjectStatus, String> {
        self.accept_action()?;
        self.live().await?;
        let mut slot = self.slot(id)?.lock().await;
        let profile = self.profile(id).await?;
        validate::require_complete(&profile)?;
        self.accept_action()?;
        self.live().await?;
        self.log(id, "info", "Запрошен запуск.");
        let result = if profile.kind == ProjectKind::Android {
            self.start_android(&profile, &mut slot, true).await
        } else {
            self.start_web(&profile, &mut slot, true).await
        };
        if let Err(message) = result {
            self.log(id, "error", &message);
            self.phase(id, Phase::Error, slot.resource.is_some(), &message)
                .await;
            return Err(message);
        }
        let status = self.status(id).await;
        if profile.kind != ProjectKind::Android && self.settings.read().await.auto_open_browser {
            process::open(&profile).await?;
        }
        Ok(status)
    }

    async fn start_web(
        &self,
        profile: &Profile,
        slot: &mut Slot,
        allow_external_reuse: bool,
    ) -> Result<(), String> {
        let probe = self.http_probe(profile).await?;
        if !allow_external_reuse && matches!(probe, Probe::Ready { .. }) {
            return Err("После остановки endpoint уже отвечает извне. Перезапуск не присваивает внешний ресурс.".into());
        }
        match start_decision(&probe, slot.resource.is_some())? {
            StartDecision::Reuse => {
                self.refresh(profile, slot).await?;
                if !matches!(
                    self.status(&profile.id).await.phase,
                    Phase::Ready | Phase::External
                ) {
                    return Err("Не подтверждена identity приложения.".into());
                }
                return Ok(());
            }
            StartDecision::Spawn => {}
        }
        self.phase(
            &profile.id,
            Phase::Starting,
            false,
            "Запускается и проверяет готовность…",
        )
        .await;
        let mut powershell_wrapper = None;
        match profile.kind {
            ProjectKind::Process => {
                let config = process::validate(profile)?;
                let bridge = self.store.vite_bridge()?;
                let (exe, args, cwd) = process::command(profile, &config, &bridge)?;
                let process = runtime::spawn_controlled(&exe, &args, &cwd)?;
                let pid = process.pid();
                slot.resource = Some(Owned::Vite { process });
                let mut status = ProjectStatus::new(
                    &profile.id,
                    Phase::Starting,
                    true,
                    "Vite запускается на настроенном loopback-порту.",
                );
                status.pid = Some(pid);
                status.started_at = Some(chrono::Utc::now().to_rfc3339());
                self.set(status).await;
            }
            ProjectKind::Powershell => {
                let config = powershell::validate(profile)?;
                let (exe, args, cwd) = powershell::command(profile, &config)?;
                powershell_wrapper = Some(runtime::spawn_managed(&exe, &args, &cwd)?);
                // The wrapper can reuse/detach another server. Never adopt it;
                // dropping this non-killing Job keeps its filesystem writes safe.
                self.log(&profile.id,"info","Запущен штатный PowerShell launcher. Владение сервером не присваивается; force-stop запрещён.");
            }
            ProjectKind::Docker => {
                let config = docker::validate(profile)?;
                docker::ensure_engine(&config).await?;
                let before = docker::inspect(&config).await?;
                if before.running {
                    return Err("Нужный Docker service уже работает, но HTTP ещё не готов. Внешнее владение сохранено; дождитесь готовности.".into());
                }
                docker::start(&config).await?;
                let after = docker::inspect(&config).await?;
                if after.container_ids.is_empty() {
                    return Err("Docker не подтвердил container identity после запуска.".into());
                }
                slot.resource = Some(Owned::Docker {
                    ids: after.container_ids,
                });
                let mut status = ProjectStatus::new(
                    &profile.id,
                    Phase::Starting,
                    true,
                    "Docker запускается; проверяется /health.",
                );
                status.started_at = Some(chrono::Utc::now().to_rfc3339());
                self.set(status).await;
            }
            ProjectKind::Android => unreachable!(),
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(wrapper) = powershell_wrapper.as_mut() {
                if wrapper_exit_result(wrapper.root_exit()?)? {
                    powershell_wrapper = None;
                }
            }
            if let Some(Owned::Vite { process }) = &mut slot.resource {
                if process.exited()? {
                    slot.resource = None;
                    return Err("Vite завершился до готовности. Проверьте совместимый Node.js и зависимости проекта; автоматическая установка отключена.".into());
                }
            }
            if matches!(self.http_probe(profile).await?, Probe::Ready { .. }) {
                self.refresh(profile, slot).await?;
                self.log(&profile.id, "info", "Готовность и identity подтверждены.");
                return Ok(());
            }
            if readiness_expired(tokio::time::Instant::now(), deadline) {
                return Err("Истекло время ожидания готовности. Повторный сервер не создаётся; управляемый ресурс сохранён для явной остановки.".into());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn start_android(
        &self,
        profile: &Profile,
        slot: &mut Slot,
        allow_external_reuse: bool,
    ) -> Result<(), String> {
        let mut config = android::validate(profile)?;
        let devices = android::list_devices(&config).await?;
        if let Some(Owned::Android { serial, .. }) = &slot.resource {
            config.serial = Some(serial.clone());
        }
        let existing = self.existing_serial(&config, &devices).await?;
        if !allow_external_reuse && existing.is_some() {
            return Err("После остановки выбранное Android-устройство уже существует извне. Перезапуск не присваивает внешний эмулятор.".into());
        }
        let target = android_target(config.serial.as_deref(), existing.as_deref(), &devices)?;
        let serial = target.serial;
        config.serial = Some(serial.clone());
        if let Some(device) = devices.iter().find(|d| d.serial == serial) {
            android::select_device(&devices, &device.serial)?;
        } else if target.new_emulator && slot.resource.is_none() {
            if config.avd.is_none() || !serial.starts_with("emulator-") {
                return Err(
                    "Выбранное устройство не подключено. Подключите его или настройте AVD.".into(),
                );
            }
            let port = serial
                .trim_start_matches("emulator-")
                .parse::<u16>()
                .map_err(|_| "Некорректный порт эмулятора.")?;
            if process::port_open(port).await || process::port_open(port.saturating_add(1)).await {
                return Err(
                    "Порт эмулятора занят. Чужой эмулятор не заменяется; выберите serial явно."
                        .into(),
                );
            }
            let (exe, args) = android::emulator_command(&config)?;
            let env = android::emulator_env(&config)?;
            let cwd =
                validate::absolute_path(config.sdk_root.as_deref().ok_or("Укажите SDK root.")?)?;
            let emulator = runtime::spawn_managed_env(&exe, &args, &cwd, &env)?;
            slot.resource = Some(Owned::Android {
                serial: serial.clone(),
                app_pid: None,
                emulator: Some(emulator),
                avd: config.avd.clone(),
            });
            self.log(
                &profile.id,
                "info",
                "Запущен выбранный AVD без очистки данных.",
            );
        }
        self.phase(
            &profile.id,
            Phase::Starting,
            slot.resource.is_some(),
            "Ожидается Android sys.boot_completed=1…",
        )
        .await;
        android::wait_boot(&config, &serial, Duration::from_secs(180)).await?;
        if let Some(Owned::Android {
            emulator: Some(process),
            avd,
            ..
        }) = &mut slot.resource
        {
            if process.exited()?
                || !process.owns_listener(emulator_port(&serial)?)?
                || android::avd_name(&config, &serial).await?.as_str()
                    != avd.as_deref().unwrap_or("")
            {
                return Err("Запущенный эмулятор не подтвердил AVD identity. Владение устройством не подтверждено; запуск приложения отменён.".into());
            }
        }
        let before = android::app_pid(&config, &serial).await?;
        android::launch(&config, &serial).await?;
        let pid = android::wait_app_pid(&config, &serial, Duration::from_secs(15)).await?;
        if before.is_none() {
            if let Some(Owned::Android {
                app_pid,
                emulator: Some(_),
                ..
            }) = &mut slot.resource
            {
                *app_pid = Some(pid);
            }
        }
        let owned = slot.resource.is_some();
        let mut status = ProjectStatus::new(
            &profile.id,
            if owned { Phase::Ready } else { Phase::External },
            owned,
            "Android открыт. Установка и сборка не выполнялись.",
        );
        status.pid = Some(pid);
        status.serial = Some(serial);
        if owned {
            status.started_at = Some(chrono::Utc::now().to_rfc3339());
        }
        self.set(status).await;
        self.log(
            &profile.id,
            "info",
            "Android открыт на адресно выбранном устройстве.",
        );
        Ok(())
    }

    /// One slot lock covers stop and start. A failed graceful stop must never start a second resource.
    pub async fn restart(&self, id: &str) -> Result<ProjectStatus, String> {
        self.accept_action()?;
        self.live().await?;
        let mut slot = self.slot(id)?.try_lock().map_err(|_| {
            "Для проекта уже выполняется операция. Повторный перезапуск не поставлен в очередь."
                .to_string()
        })?;
        let profile = self.profile(id).await?;
        validate::require_complete(&profile)?;
        self.accept_action()?;
        self.live().await?;
        stop_permission(&profile.kind, slot.resource.is_some())?;
        if let Some(Owned::Vite { process }) = &mut slot.resource {
            let port = process::validate(&profile)?.port;
            if process::port_open(port).await && !process.owns_listener(port)? {
                return Err("HTTP-порт принадлежит внешнему процессу. Перезапуск не выполнялся; управляемый процесс оставлен работающим.".into());
            }
        }
        self.log(
            id,
            "info",
            "Запрошен последовательный перезапуск управляемого ресурса.",
        );
        self.phase(
            id,
            Phase::Stopping,
            true,
            "Перезапуск: ожидается корректная остановка…",
        )
        .await;
        if let Err(message) = self.stop_owned(&profile, &mut slot).await {
            self.phase(id, Phase::Error, slot.resource.is_some(), &message)
                .await;
            self.log(id, "error", &message);
            return Err(message);
        }
        slot.resource = None;
        self.phase(
            id,
            Phase::Stopped,
            false,
            "Корректная остановка подтверждена.",
        )
        .await;
        // No public stop()/start() call here: both would reacquire this same lock.
        self.accept_action()?;
        self.live().await?;
        let result = if profile.kind == ProjectKind::Android {
            self.start_android(&profile, &mut slot, false).await
        } else {
            self.start_web(&profile, &mut slot, false).await
        };
        if let Err(message) = result {
            self.phase(id, Phase::Error, slot.resource.is_some(), &message)
                .await;
            self.log(id, "error", &message);
            return Err(message);
        }
        self.log(
            id,
            "info",
            "Перезапуск завершён; готовность и identity подтверждены.",
        );
        Ok(self.status(id).await)
    }

    pub async fn diagnostics(&self, id: &str) -> Result<ProjectDiagnostics, String> {
        use DiagnosticState::{Checking, Error, NotApplicable, Ready, Unknown};
        let operation = self.slot(id)?;
        let Ok(mut slot) = operation.try_lock() else {
            return Ok(diagnostics::unavailable(
                id,
                "Выполняется операция проекта; измерения временно отложены.",
                Checking,
            ));
        };
        let mut profile = self.profile(id).await?;
        let validation = validate::profile_validation(&profile);
        if !validation.complete {
            return Ok(diagnostics::unavailable(
                id,
                &validation.issues.join(" "),
                Unknown,
            ));
        }
        if self.quitting.load(Ordering::SeqCst) {
            return Ok(diagnostics::unavailable(
                id,
                "Завершается работа Project Shelf; новые проверки не выполняются.",
                Unknown,
            ));
        }
        if !self.settings.read().await.live_actions_enabled {
            return Ok(diagnostics::unavailable(
                id,
                "Системные действия отключены в настройках. Измерения не выполнялись.",
                Unknown,
            ));
        }
        let mut result = diagnostics::unavailable(
            id,
            "Ресурс запущен вне текущего сеанса или владение не подтверждено.",
            Unknown,
        );
        let mut owned_verified = false;
        let mut restart_conflict = false;
        if profile.kind == ProjectKind::Android {
            // Only the in-memory diagnostic target changes: never persist a dynamic serial.
            if let Some(Owned::Android { serial, .. }) = &slot.resource {
                profile.config["serial"] = serde_json::json!(serial);
            }
            let observation = android::diagnose(&profile).await;
            let dependencies = diagnostics::check(
                "dependencies",
                if observation.sdk_ready { Ready } else { Error },
                observation.dependency_message,
            );
            let mut process = diagnostics::check(
                "process",
                if !observation.errors.is_empty() {
                    Error
                } else if observation.boot_completed == Some(true) && observation.app_pid.is_some()
                {
                    Ready
                } else {
                    Unknown
                },
                observation.process_message,
            );
            process.value = observation.serial.clone().map(|serial| {
                format!(
                    "{serial}; boot={}; APK установлен={}; PID={}",
                    observation
                        .boot_completed
                        .map(|v| if v { "1" } else { "0" })
                        .unwrap_or("не проверено"),
                    observation
                        .app_installed
                        .map(|v| if v { "да" } else { "нет" })
                        .unwrap_or("не проверено"),
                    observation
                        .app_pid
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "не найден/не проверен".into())
                )
            });
            if let Some(Owned::Android {
                serial,
                emulator: Some(emulator),
                app_pid,
                ..
            }) = &mut slot.resource
            {
                owned_verified = observation.serial.as_deref() == Some(serial.as_str())
                    && !emulator.exited()?
                    && emulator.owns_listener(emulator_port(serial)?)?
                    && app_pid.map_or(true, |expected| {
                        observation
                            .app_pid
                            .map_or(true, |actual| expected == actual)
                    });
            }
            result.technologies = if observation.sdk_ready {
                vec!["Android SDK / ADB (файлы найдены)".into()]
            } else {
                Vec::new()
            };
            result.checks = vec![
                dependencies,
                process,
                diagnostics::check(
                    "http",
                    NotApplicable,
                    "Android проверяется через ADB, без HTTP endpoint.",
                ),
                diagnostics::check(
                    "port",
                    NotApplicable,
                    "Порт веб-сервера не применяется. Устройство выбирается по конкретному serial.",
                ),
            ];
        } else {
            let observation = diagnostics::web(&self.client, &profile).await?;
            let mut process = diagnostics::check(
                "process",
                Unknown,
                "Процесс вне текущего сеанса: PID и владение не подтверждены.",
            );
            result.technologies = observation.technologies;
            let mut dependencies = observation.dependencies;
            match &mut slot.resource {
                Some(Owned::Vite { process: managed }) => {
                    let alive = !managed.exited()?;
                    owned_verified = alive;
                    process = diagnostics::check(
                        "process",
                        if alive { Ready } else { Error },
                        if alive {
                            "Процесс существует в Windows Job текущего сеанса."
                        } else {
                            "Управляемый процесс завершился. Повторный запуск автоматически не выполнялся."
                        },
                    );
                    process.value = Some(format!("PID {}", managed.pid()));
                    if alive
                        && matches!(observation.port.state, Ready | Error)
                        && !managed.owns_listener(observation.port_number)?
                    {
                        restart_conflict = true;
                        process.state = Error;
                        process.message="Процесс управляется Shelf, но HTTP-порт не принадлежит его Windows Job. Чужой endpoint не присваивается.".into();
                    }
                }
                _ => {}
            }
            if profile.kind == ProjectKind::Docker {
                let docker = docker::diagnose(&profile).await;
                dependencies = diagnostics::check(
                    "dependencies",
                    if docker.cli_available && docker.engine_ready == Some(true) {
                        Ready
                    } else if !docker.cli_available || docker.engine_ready == Some(false) {
                        Error
                    } else {
                        Unknown
                    },
                    docker.dependency_message,
                );
                process = diagnostics::check(
                    "process",
                    if !docker.cli_available || docker.engine_ready != Some(true) {
                        Unknown
                    } else if docker.identity_verified && docker.running == Some(true) {
                        Ready
                    } else if docker.running == Some(false) {
                        Unknown
                    } else {
                        Error
                    },
                    docker.process_message,
                );
                if !docker.container_ids.is_empty() {
                    process.value = Some(
                        docker
                            .container_ids
                            .iter()
                            .map(|id| id.chars().take(12).collect::<String>())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                if let Some(Owned::Docker { ids }) = &slot.resource {
                    owned_verified = docker.identity_verified
                        && docker::matching_container_ids(ids, &docker.container_ids);
                }
                result.technologies = vec!["Docker / Compose".into()];
            } else if profile.kind == ProjectKind::Powershell {
                process = diagnostics::check(
                    "process",
                    Unknown,
                    if observation.identity_verified {
                        "Health identity подтверждена. Сообщённый сервером PID не даёт Shelf владения процессом."
                    } else {
                        "Процесс PowerShell-сайта не подтверждён health identity."
                    },
                );
                process.value = observation
                    .reported_pid
                    .map(|pid| format!("Health сообщает PID {pid}"));
                owned_verified = false;
            }
            result.checks = vec![dependencies, process, observation.http, observation.port];
        }
        let permission = stop_permission(&profile.kind, owned_verified);
        result.can_stop = permission.is_ok();
        result.can_restart = permission.is_ok();
        result.stop_reason = permission.as_ref().err().cloned();
        result.restart_reason = permission.err();
        if restart_conflict {
            result.can_restart = false;
            result.restart_reason = Some(
                "HTTP-порт принадлежит другому процессу. Перезапуск не изменяет внешние ресурсы."
                    .into(),
            );
        }
        // checked_at marks the beginning of this observation pass, so slow tools cannot make old HTTP data look newer.
        Ok(result)
    }

    pub async fn stop(&self, id: &str) -> Result<ProjectStatus, String> {
        self.live().await?;
        let mut slot = self.slot(id)?.lock().await;
        let profile = self.profile(id).await?;
        stop_permission(&profile.kind, slot.resource.is_some())?;
        self.phase(
            id,
            Phase::Stopping,
            true,
            "Останавливается только управляемый ресурс…",
        )
        .await;
        let result = self.stop_owned(&profile, &mut slot).await;
        if let Err(message) = result {
            self.phase(id, Phase::Error, slot.resource.is_some(), &message)
                .await;
            self.log(id, "error", &message);
            return Err(message);
        }
        slot.resource = None;
        self.phase(id, Phase::Stopped, false, "Остановлен. Данные сохранены.")
            .await;
        self.log(
            id,
            "info",
            "Управляемый ресурс остановлен без удаления данных.",
        );
        Ok(self.status(id).await)
    }
    async fn stop_owned(&self, profile: &Profile, slot: &mut Slot) -> Result<(), String> {
        match slot
            .resource
            .as_mut()
            .ok_or("Нет подтверждённого владения.")?
        {
            Owned::Vite { process } => {
                process.graceful_shutdown(Duration::from_secs(15)).await?;
            }
            Owned::Docker { ids } => {
                docker::stop(&docker::validate(profile)?, ids).await?;
            }
            Owned::Android {
                serial,
                app_pid,
                emulator,
                avd,
            } => {
                let config = android::validate(profile)?;
                let own_emulator=emulator.as_mut().ok_or("Внешнее Android-устройство не останавливается, даже если Shelf открыл приложение.")?;
                if own_emulator.exited()? {
                    return Ok(());
                }
                if !own_emulator.owns_listener(emulator_port(serial)?)?
                    || android::avd_name(&config, serial).await?.as_str()
                        != avd.as_deref().unwrap_or("")
                {
                    return Err("Порт и AVD не подтверждают принадлежность эмулятора текущему Windows Job. Остановка запрещена.".into());
                }
                if let Some(expected) = *app_pid {
                    let actual = android::app_pid(&config, serial).await?;
                    if actual.is_some() && actual != Some(expected) {
                        return Err(
                            "PID Android изменился; внешний процесс не останавливается.".into()
                        );
                    }
                    if actual.is_some() {
                        android::force_stop(&config, serial).await?;
                    }
                    *app_pid = None;
                }
                if let Some(process) = emulator {
                    if !process.exited()? {
                        if android::avd_name(&config, serial).await?.as_str()
                            != avd.as_deref().unwrap_or("")
                        {
                            return Err("AVD identity изменилась; завершение запрещено.".into());
                        }
                        android::stop_emulator(&config, serial).await?;
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
                        while !process.exited()? {
                            if tokio::time::Instant::now() >= deadline {
                                return Err("Эмулятор ещё завершается. Принудительная остановка не выполняется.".into());
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub async fn open(&self, id: &str) -> Result<ProjectStatus, String> {
        self.accept_action()?;
        let mut slot = self.slot(id)?.lock().await;
        let profile = self.profile(id).await?;
        validate::require_complete(&profile)?;
        self.accept_action()?;
        self.refresh(&profile, &mut slot).await?;
        let status = self.status(id).await;
        if !matches!(status.phase, Phase::Ready | Phase::External) {
            return Err("Приложение ещё не готово; сначала запустите его.".into());
        }
        if profile.kind == ProjectKind::Android {
            self.live().await?;
            let config = android::validate(&profile)?;
            let serial = status.serial.as_deref().ok_or("Выберите Android serial.")?;
            android::launch(&config, serial).await?;
        } else {
            process::open(&profile).await?;
        }
        Ok(status)
    }
    pub async fn android_devices(&self) -> Result<Vec<android::AndroidDevice>, String> {
        self.accept_action()?;
        self.live().await?;
        let _op = self.slot("android")?.lock().await;
        let profile = self.profile("android").await?;
        validate::require_complete(&profile)?;
        self.accept_action()?;
        android::list_devices(&android::validate(&profile)?).await
    }
    pub async fn install_android(&self, id: &str, serial: &str) -> Result<(), String> {
        self.accept_action()?;
        self.live().await?;
        let _op = self.slot(id)?.lock().await;
        let profile = self.profile(id).await?;
        validate::require_complete(&profile)?;
        if profile.kind != ProjectKind::Android {
            return Err("Установка APK доступна только Android.".into());
        }
        self.accept_action()?;
        self.live().await?;
        android::validate_serial(serial)?;
        android::install(&android::validate(&profile)?, serial).await?;
        self.log(id, "info", "APK установлен с -r; данные не очищались.");
        Ok(())
    }
    pub async fn quit_summary(&self, stop_owned: bool) -> Result<QuitSummary, String> {
        if self
            .quitting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("Выход уже выполняется.".into());
        }
        // Wait for all operations that passed the gate. New mutations recheck
        // it after taking their slot, so no resource can appear after snapshot.
        let mut managed = Vec::new();
        for id in validate::ALLOWED_IDS {
            let slot = self.slots[id].lock().await;
            if slot.resource.is_some() {
                let mut status = self.status(id).await;
                status.owned = true;
                managed.push(status);
            }
        }
        let mut failed = Vec::new();
        if stop_owned {
            for status in &managed {
                if self.stop(&status.id).await.is_err() {
                    failed.push(status.id.clone());
                }
            }
        }
        let mut retained = Vec::new();
        for profile in self.profiles().await {
            let status = self.status(&profile.id).await;
            if status.owned
                || matches!(
                    status.phase,
                    Phase::Ready | Phase::External | Phase::Starting
                )
            {
                retained.push(profile.id.clone());
            }
        }
        if stop_owned && (!retained.is_empty() || !failed.is_empty()) {
            self.quitting.store(false, Ordering::SeqCst);
        }
        Ok(QuitSummary {
            managed,
            retained,
            failed,
        })
    }
    pub fn mark_exiting(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn isolated_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "project-shelf-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }
    #[tokio::test]
    async fn production_setup_guards_reject_actions_before_every_native_boundary() {
        let before = runtime::native_effect_attempts();
        let root = isolated_root("setup");
        let backend = Supervisor::new(root.clone()).unwrap();
        let statuses = backend.statuses().await;
        assert_eq!(statuses.len(), 4);
        assert!(statuses.iter().all(|s| s.phase == Phase::Setup && !s.owned));
        for id in validate::ALLOWED_IDS {
            assert!(backend.start(id).await.is_err());
            assert!(backend.open(id).await.is_err());
            assert!(backend.restart(id).await.is_err());
            let measured = backend.diagnostics(id).await.unwrap();
            assert!(measured
                .checks
                .iter()
                .all(|c| c.state == DiagnosticState::Unknown && c.http_status.is_none()));
            assert!(
                measured.technologies.is_empty() && !measured.can_stop && !measured.can_restart
            );
        }
        assert!(backend
            .install_android("android", "emulator-5554")
            .await
            .is_err());
        assert!(backend.android_devices().await.is_err());
        assert_eq!(
            runtime::native_effect_attempts(),
            before,
            "incomplete profiles must not reach TCP, browser or process boundaries"
        );
        std::fs::remove_dir(root).unwrap();
    }
    #[tokio::test]
    async fn typed_partial_config_roundtrips_and_active_edits_are_refused() {
        let root = isolated_root("config");
        let backend = Supervisor::new(root.clone()).unwrap();
        let mut profile = backend.profile("web").await.unwrap();
        profile.path = r"C:\fixtures\web".into();
        backend.save_profile(profile.clone()).await.unwrap();
        assert!(!validate::profile_validation(&profile).complete);
        let restored = Supervisor::new(root.clone()).unwrap();
        assert_eq!(restored.profile("web").await.unwrap().path, profile.path);
        let current = backend.profile("docker").await.unwrap();
        {
            backend.slots["docker"].lock().await.resource = Some(Owned::Docker {
                ids: vec!["a".repeat(64)],
            });
        }
        let mut edited = current.clone();
        edited.config["service"] = serde_json::json!("sample-service");
        assert!(backend.save_profile(edited).await.is_err());
        let mut favorite = current;
        favorite.favorite = !favorite.favorite;
        backend.save_profile(favorite).await.unwrap();
        std::fs::remove_file(root.join("profiles.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn diagnostic_busy_slot_and_external_restart_never_probe_or_launch() {
        let root = std::env::temp_dir().join(format!(
            "project-shelf-readiness-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let backend = Supervisor::new(root.clone()).unwrap();
        // Complete synthetic fixtures prove the ownership gate, rather than merely failing setup.
        {
            let mut profiles = backend.profiles.write().await;
            for profile in profiles.iter_mut() {
                match profile.kind {
                    ProjectKind::Process => {
                        profile.path = r"C:\fixtures\web".into();
                        profile.config["readyMarker"] = serde_json::json!("Fixture");
                    }
                    ProjectKind::Powershell => {
                        profile.path = r"C:\fixtures\site".into();
                        profile.config["service"] = serde_json::json!("fixture-service");
                    }
                    ProjectKind::Docker => {
                        profile.config["composeFile"] =
                            serde_json::json!(r"C:\fixtures\compose.yml");
                        profile.config["service"] = serde_json::json!("fixture-service");
                        profile.config["healthMessage"] = serde_json::json!("fixture alive");
                    }
                    ProjectKind::Android => {
                        profile.config["sdkRoot"] = serde_json::json!(r"C:\fixtures\sdk");
                        profile.config["avd"] = serde_json::json!("Fixture_Device");
                        profile.config["packageName"] = serde_json::json!("com.example.app");
                        profile.config["activity"] = serde_json::json!(".MainActivity");
                    }
                }
                assert!(validate::profile_validation(profile).complete);
            }
        }
        let effects_before = runtime::native_effect_attempts();
        let id = validate::ALLOWED_IDS[0];
        {
            let _operation = backend.slots[id].lock().await;
            let measured = tokio::time::timeout(Duration::from_secs(2), backend.diagnostics(id))
                .await
                .expect("busy diagnostics must not queue")
                .unwrap();
            assert!(measured
                .checks
                .iter()
                .all(|c| c.state == DiagnosticState::Checking && c.http_status.is_none()));
            assert!(!measured.can_restart && !measured.can_stop);
            assert!(
                tokio::time::timeout(Duration::from_secs(2), backend.restart(id))
                    .await
                    .expect("busy restart must not queue")
                    .is_err(),
                "duplicate operation must be rejected, never queued"
            );
        }
        for id in validate::ALLOWED_IDS {
            assert!(
                backend.restart(id).await.is_err(),
                "external/unowned slot cannot be restarted"
            );
            assert!(backend.slots[id].lock().await.resource.is_none());
        }
        {
            backend.slots["powershell"].lock().await.resource = Some(Owned::Docker {
                ids: vec!["a".repeat(64)],
            });
            assert!(
                backend.restart("powershell").await.is_err(),
                "PowerShell stop/restart is unsupported even with a registry entry"
            );
            assert!(backend.slots["powershell"].lock().await.resource.is_some());
        }
        assert_eq!(runtime::native_effect_attempts(), effects_before);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn wrapper_failure_is_immediate_but_successful_detach_keeps_readiness_wait() {
        assert!(!wrapper_exit_result(None).unwrap());
        assert!(wrapper_exit_result(Some((true, Some(0)))).unwrap());
        let error = wrapper_exit_result(Some((false, Some(1)))).unwrap_err();
        assert!(error.contains("код 1"));
    }
    #[test]
    fn default_android_reuses_only_matched_avd_as_external() {
        let devices = android::parse_devices("emulator-5556 device\n");
        assert_eq!(
            android_target(None, Some("emulator-5556"), &devices).unwrap(),
            AndroidTarget {
                serial: "emulator-5556".into(),
                new_emulator: false
            }
        );
        assert_eq!(
            android_target(Some("emulator-5556"), None, &devices)
                .unwrap()
                .new_emulator,
            false
        );
    }
    #[test]
    fn wrong_avd_on_default_port_is_a_conflict() {
        let devices = android::parse_devices("emulator-5554 device\n");
        assert!(android_target(None, None, &devices).is_err());
        assert_eq!(
            android_target(None, None, &[]).unwrap(),
            AndroidTarget {
                serial: "emulator-5554".into(),
                new_emulator: true
            }
        );
    }
    #[tokio::test]
    async fn quit_waits_for_inflight_operation_and_never_kills_when_leaving() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-data")
            .join(format!(
                "quit-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
        let backend = std::sync::Arc::new(Supervisor::new(root.clone()).unwrap());
        let mut in_flight = backend.slots["docker"].lock().await;
        let clone = backend.clone();
        let pending = tokio::spawn(async move { clone.quit_summary(false).await });
        while !backend.quitting.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        assert!(!pending.is_finished());
        assert!(backend.accept_action().is_err());
        in_flight.resource = Some(Owned::Docker {
            ids: vec!["a".repeat(64)],
        });
        backend.phase("docker", Phase::Ready, true, "test").await;
        drop(in_flight);
        let summary = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(summary.managed.len(), 1);
        assert_eq!(summary.retained, vec!["docker"]);
        assert!(backend.slots["docker"].lock().await.resource.is_some());
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn duplicate_start_never_spawns_while_owned_resource_waits() {
        assert!(start_decision(&Probe::Missing, true).is_err());
        assert_eq!(
            start_decision(&Probe::Ready { pid: None }, true).unwrap(),
            StartDecision::Reuse
        );
    }
    #[test]
    fn external_ready_response_is_reused_without_adoption() {
        assert_eq!(
            start_decision(&Probe::Ready { pid: Some(42) }, false).unwrap(),
            StartDecision::Reuse
        );
        assert!(stop_permission(&ProjectKind::Process, false).is_err());
    }
    #[test]
    fn busy_foreign_port_cannot_spawn_or_be_stopped() {
        assert!(start_decision(&Probe::Conflict("foreign".into()), false).is_err());
        assert!(stop_permission(&ProjectKind::Docker, false).is_err());
    }
    #[test]
    fn powershell_force_stop_always_forbidden() {
        assert!(stop_permission(&ProjectKind::Powershell, true).is_err());
        assert!(stop_permission(&ProjectKind::Powershell, false).is_err());
    }
    #[test]
    fn readiness_timeout_is_finite_and_owned_resource_does_not_restart() {
        let now = tokio::time::Instant::now();
        assert!(!readiness_expired(now, now + Duration::from_secs(1)));
        assert!(readiness_expired(
            now + Duration::from_secs(2),
            now + Duration::from_secs(1)
        ));
        assert!(start_decision(&Probe::Missing, true).is_err());
    }
}
