//! Narrow Android adapter. Every target action uses an explicit, verified serial.
//! This module never builds, wipes, uninstalls, clears app data, or stops ADB.
//! The supervisor must enforce the live-action gate and ownership before stop.

use std::{future::Future, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{models::Profile, runtime::run_tool};

#[path = "../diagnostics_android.rs"]
mod diagnostics;
pub use diagnostics::{diagnose, AndroidDiagnosticsObservation};

const TOOL_TIMEOUT: Duration = Duration::from_secs(8);
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidConfig {
    #[serde(default)]
    pub sdk_root: Option<String>,
    #[serde(default)]
    pub avd_home: Option<String>,
    #[serde(default)]
    pub android_user_home: Option<String>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub avd: Option<String>,
    #[serde(default)]
    pub package_name: String,
    #[serde(default)]
    pub activity: String,
    #[serde(default)]
    pub apk_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String,
    pub label: String,
}

#[cfg(test)]
fn default_config() -> AndroidConfig {
    AndroidConfig {
        sdk_root: None,
        avd_home: None,
        android_user_home: None,
        serial: None,
        avd: None,
        package_name: "com.example.app".into(),
        activity: ".MainActivity".into(),
        apk_path: None,
    }
}

pub fn validate(profile: &Profile) -> Result<AndroidConfig, String> {
    let mut config: AndroidConfig = serde_json::from_value(profile.config.clone())
        .map_err(|_| "Некорректная конфигурация Android-профиля.".to_string())?;
    for value in [
        &mut config.sdk_root,
        &mut config.avd_home,
        &mut config.android_user_home,
        &mut config.serial,
        &mut config.avd,
        &mut config.apk_path,
    ] {
        *value = value.take().filter(|text| !text.trim().is_empty());
    }
    if config.package_name.trim().is_empty() {
        config.package_name.clear();
    }
    if config.activity.trim().is_empty() {
        config.activity.clear();
    }
    validate_fields(&config)?;
    Ok(config)
}

/// Validate present fields only. Storage may safely keep an incomplete profile;
/// actions must additionally pass `require_complete` before touching tools.
fn validate_fields(config: &AndroidConfig) -> Result<(), String> {
    if !config.package_name.trim().is_empty() {
        validate_package(&config.package_name)?;
    }
    if !config.activity.trim().is_empty() {
        validate_activity(&config.activity)?;
    }
    if let Some(serial) = &config.serial {
        validate_serial(serial)?;
    }
    if let Some(avd) = &config.avd {
        validate_avd(avd)?;
    }
    if let Some(root) = &config.sdk_root {
        validate_absolute_path(root, "Android SDK")?;
    }
    if let Some(root) = &config.avd_home {
        validate_absolute_path(root, "AVD")?;
    }
    if let Some(root) = &config.android_user_home {
        validate_absolute_path(root, "Android user home")?;
    }
    // Pure lexical checks only: saving setup does not inspect user files or SDK.
    if let Some(apk) = &config.apk_path {
        let path = validate_absolute_path(apk, "APK")?;
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("apk"))
        {
            return Err("Для установки выберите файл .apk.".into());
        }
    }
    Ok(())
}

fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|text| !text.trim().is_empty())
}

/// Missing setup fields, without filesystem, environment, network, or CLI access.
pub fn issues(config: &AndroidConfig) -> Vec<String> {
    let mut issues = Vec::new();
    if present(&config.sdk_root).is_none() {
        issues.push("Укажите путь Android SDK (sdkRoot).".into());
    }
    if config.package_name.trim().is_empty() {
        issues.push("Укажите packageName приложения.".into());
    }
    if config.activity.trim().is_empty() {
        issues.push("Укажите Activity приложения.".into());
    }
    if present(&config.avd).is_none() && present(&config.serial).is_none() {
        issues.push("Укажите имя AVD или явный serial устройства.".into());
    }
    issues
}

fn require_complete(config: &AndroidConfig) -> Result<(), String> {
    let missing = issues(config);
    if !missing.is_empty() {
        return Err(format!("Настройте Android-профиль: {}", missing.join(" ")));
    }
    validate_fields(config)
}

fn validate_absolute_path(value: &str, label: &str) -> Result<PathBuf, String> {
    crate::validate::absolute_path(value).map_err(|_| {
        format!("Укажите корректный абсолютный локальный путь {label} без потоков данных, шаблонов и переходов.")
    })
}

pub fn validate_serial(serial: &str) -> Result<(), String> {
    if serial.is_empty()
        || serial.len() > 128
        || !serial.as_bytes()[0].is_ascii_alphanumeric()
        || !serial
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
    {
        return Err(
            "Некорректный Android serial. Выберите точный serial из списка устройств.".into(),
        );
    }
    Ok(())
}

fn java_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .as_bytes()
                    .first()
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}

pub fn validate_component(package: &str, activity: &str) -> Result<(), String> {
    validate_package(package)?;
    validate_activity(activity)
}

fn validate_package(package: &str) -> Result<(), String> {
    if package.len() > 255 || !package.contains('.') || !java_name(package) {
        return Err("Некорректный packageName Android-приложения.".into());
    }
    Ok(())
}

fn validate_activity(activity: &str) -> Result<(), String> {
    if activity.len() > 255 || !java_name(activity.strip_prefix('.').unwrap_or(activity)) {
        return Err("Некорректная Activity Android-приложения.".into());
    }
    Ok(())
}

fn validate_avd(avd: &str) -> Result<(), String> {
    if avd.is_empty()
        || avd.len() > 128
        || !avd.as_bytes()[0].is_ascii_alphanumeric()
        || !avd
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err("Некорректное имя AVD. Укажите существующий AVD из Android SDK.".into());
    }
    Ok(())
}

fn sdk_root(config: &AndroidConfig) -> Result<PathBuf, String> {
    let configured = present(&config.sdk_root).ok_or_else(|| {
        "Укажите явный sdkRoot в Android-профиле; автоматический поиск SDK отключён.".to_string()
    })?;
    validate_absolute_path(configured, "Android SDK")
}

pub fn adb_path(config: &AndroidConfig) -> Result<String, String> {
    require_complete(config)?;
    let file = sdk_root(config)?
        .join("platform-tools")
        .join(if cfg!(windows) { "adb.exe" } else { "adb" });
    if !file.is_file() {
        return Err(
            "Не найден platform-tools/adb в Android SDK. Автоматическая установка отключена."
                .into(),
        );
    }
    Ok(file.to_string_lossy().into_owned())
}

pub fn parse_devices(stdout: &str) -> Vec<AndroidDevice> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            if serial == "List" || serial.starts_with('*') || validate_serial(serial).is_err() {
                return None;
            }
            let state = fields.next()?;
            if !matches!(
                state,
                "device" | "offline" | "unauthorized" | "recovery" | "sideload" | "bootloader"
            ) {
                return None;
            }
            let model = fields
                .find_map(|field| field.strip_prefix("model:"))
                .map(|model| model.chars().take(80).collect::<String>().replace('_', " "));
            let label = model.unwrap_or_else(|| {
                if serial.starts_with("emulator-") {
                    "Android Emulator".into()
                } else {
                    "Android".into()
                }
            });
            Some(AndroidDevice {
                serial: serial.to_string(),
                state: state.to_string(),
                label,
            })
        })
        .collect()
}

pub fn select_device<'a>(
    devices: &'a [AndroidDevice],
    serial: &str,
) -> Result<&'a AndroidDevice, String> {
    validate_serial(serial)?;
    let mut matches = devices.iter().filter(|device| device.serial == serial);
    let found = matches.next().ok_or_else(|| {
        "Выбранное устройство не подключено. Serial не будет заменён автоматически.".to_string()
    })?;
    if matches.next().is_some() {
        return Err("ADB вернул неоднозначный serial; действие отменено.".into());
    }
    match found.state.as_str() {
        "device" => Ok(found),
        "unauthorized" => Err(
            "Устройство не авторизовано. Разрешите USB debugging на выбранном устройстве.".into(),
        ),
        "offline" => Err("Выбранное Android-устройство offline.".into()),
        _ => Err("Выбранное устройство не находится в обычном режиме Android.".into()),
    }
}

pub async fn list_devices(config: &AndroidConfig) -> Result<Vec<AndroidDevice>, String> {
    require_complete(config)?;
    let executable = adb_path(config)?;
    let output = run_tool(
        &executable,
        &["devices".into(), "-l".into()],
        None,
        TOOL_TIMEOUT,
    )
    .await?;
    if !output.success {
        return Err(
            "ADB не смог получить список устройств. Проверьте Android SDK и USB debugging.".into(),
        );
    }
    Ok(parse_devices(&output.stdout))
}

async fn target_tool(
    config: &AndroidConfig,
    serial: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<crate::runtime::ToolOutput, String> {
    require_complete(config)?;
    validate_serial(serial)?;
    let executable = adb_path(config)?;
    let mut argv = vec!["-s".into(), serial.into()];
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    run_tool(&executable, &argv, None, timeout).await
}

pub async fn wait_boot(
    config: &AndroidConfig,
    serial: &str,
    timeout: Duration,
) -> Result<(), String> {
    require_complete(config)?;
    validate_serial(serial)?;
    // Resolve once before waiting, so an absent SDK fails immediately.
    adb_path(config)?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("Истекло время ожидания Android: sys.boot_completed не стал равен 1. Данные не изменены.".into());
        }
        let listing = tokio::time::timeout(remaining.min(TOOL_TIMEOUT), list_devices(config)).await;
        if let Ok(Ok(devices)) = listing {
            if let Some(device) = devices.iter().find(|device| device.serial == serial) {
                if device.state == "unauthorized" {
                    return Err("Устройство не авторизовано. Подтвердите USB debugging.".into());
                }
                if device.state == "device" {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if !remaining.is_zero() {
                        if let Ok(output) = target_tool(
                            config,
                            serial,
                            &["shell", "getprop", "sys.boot_completed"],
                            remaining.min(TOOL_TIMEOUT),
                        )
                        .await
                        {
                            if output.success && output.stdout.trim() == "1" {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if !remaining.is_zero() {
            tokio::time::sleep(remaining.min(Duration::from_millis(500))).await;
        }
    }
}

pub async fn app_pid(config: &AndroidConfig, serial: &str) -> Result<Option<u32>, String> {
    require_complete(config)?;
    validate_component(&config.package_name, &config.activity)?;
    select_device(&list_devices(config).await?, serial)?;
    let output = target_tool(
        config,
        serial,
        &["shell", "pidof", &config.package_name],
        TOOL_TIMEOUT,
    )
    .await?;
    parse_app_pid(
        output.success,
        output.exit_code,
        &output.stdout,
        &output.stderr,
    )
}

fn parse_app_pid(
    success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<Option<u32>, String> {
    if !success && exit_code == Some(1) && stdout.trim().is_empty() && stderr.trim().is_empty() {
        // Only pidof's exact "no process" result establishes absence. Transport
        // failures must never grant ownership of an already-running app.
        return Ok(None);
    }
    if !success || exit_code != Some(0) || !stderr.trim().is_empty() {
        return Err(
            "Не удалось проверить процесс Android-приложения. Владение не подтверждено.".into(),
        );
    }
    let mut pids = stdout.split_whitespace();
    let first = pids.next().ok_or_else(|| {
        "ADB не подтвердил наличие или отсутствие процесса приложения.".to_string()
    })?;
    let pid: u32 = first
        .parse()
        .map_err(|_| "ADB вернул некорректный PID приложения.".to_string())?;
    if pid == 0 || pids.next().is_some() {
        return Err("Процесс приложения неоднозначен. Владение не подтверждено.".into());
    }
    Ok(Some(pid))
}

/// `am start` may return before Android has created the app process. Confirm
/// the PID with a finite readiness poll instead of treating that gap as failure.
/// The supervisor still decides ownership; observing a PID alone does not grant it.
pub async fn wait_app_pid(
    config: &AndroidConfig,
    serial: &str,
    timeout: Duration,
) -> Result<u32, String> {
    require_complete(config)?;
    validate_component(&config.package_name, &config.activity)?;
    validate_serial(serial)?;
    poll_app_pid(
        timeout.min(Duration::from_secs(15)),
        Duration::from_millis(250),
        || app_pid(config, serial),
    )
    .await
}

/// The same polling policy is used by production and injected-probe tests.
/// Each probe shares the remaining total budget; a stalled probe cannot extend it.
async fn poll_app_pid<F, Fut>(
    timeout: Duration,
    interval: Duration,
    mut probe: F,
) -> Result<u32, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<u32>, String>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let timeout_error = || {
        "Истекло время ожидания процесса приложения после открытия Activity. PID не подтверждён; владение не присвоено."
            .to_string()
    };
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timeout_error());
        }
        match tokio::time::timeout(remaining, probe()).await {
            Ok(Ok(Some(pid))) if pid != 0 => return Ok(pid),
            Ok(Ok(Some(_))) => return Err("ADB вернул некорректный PID приложения.".into()),
            Ok(Ok(None)) => {}
            // A transport failure is not an absent app; do not hide it by retrying.
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(timeout_error()),
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timeout_error());
        }
        tokio::time::sleep(remaining.min(interval)).await;
    }
}

pub async fn launch(config: &AndroidConfig, serial: &str) -> Result<(), String> {
    require_complete(config)?;
    validate_component(&config.package_name, &config.activity)?;
    wait_boot(config, serial, BOOT_TIMEOUT).await?;
    select_device(&list_devices(config).await?, serial)?;
    let component = format!("{}/{}", config.package_name, config.activity);
    let output = target_tool(
        config,
        serial,
        &["shell", "am", "start", "-n", &component],
        TOOL_TIMEOUT,
    )
    .await?;
    if !output.success || output.stdout.contains("Error:") || output.stderr.contains("Error:") {
        return Err("Android не смог открыть указанную Activity. Проверьте, что приложение установлено и компонент указан верно.".into());
    }
    Ok(())
}

/// Only call after the supervisor has verified its launch ownership/PID.
pub async fn force_stop(config: &AndroidConfig, serial: &str) -> Result<(), String> {
    require_complete(config)?;
    validate_component(&config.package_name, &config.activity)?;
    select_device(&list_devices(config).await?, serial)?;
    let output = target_tool(
        config,
        serial,
        &["shell", "am", "force-stop", &config.package_name],
        TOOL_TIMEOUT,
    )
    .await?;
    if !output.success {
        return Err("Не удалось остановить принадлежащее Project Shelf Android-приложение.".into());
    }
    Ok(())
}

pub async fn install(config: &AndroidConfig, serial: &str) -> Result<(), String> {
    require_complete(config)?;
    validate_component(&config.package_name, &config.activity)?;
    validate_serial(serial)?;
    let configured = config.apk_path.as_deref().ok_or_else(|| {
        "Сначала укажите APK в настройках профиля. Сборка APK автоматически не выполняется."
            .to_string()
    })?;
    let path = validate_absolute_path(configured, "APK")?;
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("apk"))
        || !path.is_file()
    {
        return Err("APK не найден. Выберите существующий локальный файл .apk.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Не удалось прочитать выбранный APK.".to_string())?;
    let apk = canonical.to_string_lossy().into_owned();
    wait_boot(config, serial, BOOT_TIMEOUT).await?;
    select_device(&list_devices(config).await?, serial)?;
    let output = target_tool(
        config,
        serial,
        &["install", "-r", &apk],
        Duration::from_secs(120),
    )
    .await?;
    if !output.success || !output.stdout.lines().any(|line| line.trim() == "Success") {
        return Err("APK не установлен. Возможна несовместимая подпись или версия; удаление приложения и очистка данных не выполнялись.".into());
    }
    Ok(())
}

fn emulator_port(serial: &str) -> Result<u16, String> {
    validate_serial(serial)?;
    let port = serial
        .strip_prefix("emulator-")
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| (5554..=5682).contains(port) && port % 2 == 0)
        .ok_or_else(|| {
            "Для запуска AVD нужен явный serial вида emulator-5554 с чётным портом 5554–5682."
                .to_string()
        })?;
    // Avoid aliases such as emulator-05554 that do not match the ADB identity.
    if serial != format!("emulator-{port}") {
        return Err("Serial эмулятора должен точно соответствовать emulator-<порт>.".into());
    }
    Ok(port)
}

pub fn emulator_command(config: &AndroidConfig) -> Result<(String, Vec<String>), String> {
    require_complete(config)?;
    let serial = config
        .serial
        .as_deref()
        .ok_or_else(|| "Выберите явный serial Android перед запуском эмулятора.".to_string())?;
    let port = emulator_port(serial)?;
    let avd = present(&config.avd).ok_or_else(|| "Устройство не подключено. Укажите существующий AVD в профиле; создание AVD автоматически не выполняется.".to_string())?;
    validate_avd(avd)?;
    let executable = sdk_root(config)?.join("emulator").join(if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    });
    if !executable.is_file() {
        return Err("Android Emulator не найден в SDK. Установка инструментов и создание AVD автоматически не выполняются.".into());
    }
    Ok((
        executable.to_string_lossy().into_owned(),
        vec!["-avd".into(), avd.into(), "-port".into(), port.to_string()],
    ))
}

/// A fixed whitelist for this emulator child only; never mutate the launcher's
/// environment or accept arbitrary user-supplied environment variable names.
pub fn emulator_env(config: &AndroidConfig) -> Result<Vec<(String, String)>, String> {
    require_complete(config)?;
    let sdk = sdk_root(config)?.to_string_lossy().into_owned();
    let mut values = vec![
        ("ANDROID_SDK_ROOT".into(), sdk.clone()),
        ("ANDROID_HOME".into(), sdk),
    ];
    if let Some(root) = &config.avd_home {
        validate_absolute_path(root, "AVD")?;
        values.push(("ANDROID_AVD_HOME".into(), root.clone()));
    }
    if let Some(root) = &config.android_user_home {
        validate_absolute_path(root, "Android user home")?;
        values.push(("ANDROID_USER_HOME".into(), root.clone()));
    }
    Ok(values)
}

fn parse_avd_name(stdout: &str) -> Result<String, String> {
    let name = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "OK")
        .ok_or_else(|| {
            "Эмулятор не сообщил имя AVD. Принадлежность не подтверждена.".to_string()
        })?;
    validate_avd(name).map_err(|_| {
        "Эмулятор сообщил некорректное имя AVD. Принадлежность не подтверждена.".to_string()
    })?;
    Ok(name.to_string())
}

/// Serial alone is insufficient to identify a previously running emulator.
/// Match the returned AVD name before reusing the selected target.
pub async fn avd_name(config: &AndroidConfig, serial: &str) -> Result<String, String> {
    require_complete(config)?;
    emulator_port(serial)?;
    select_device(&list_devices(config).await?, serial)?;
    let output = target_tool(config, serial, &["emu", "avd", "name"], TOOL_TIMEOUT).await?;
    if !output.success {
        return Err(
            "Не удалось проверить имя AVD выбранного эмулятора. Принадлежность не подтверждена."
                .into(),
        );
    }
    parse_avd_name(&output.stdout)
}

/// The supervisor must verify this exact emulator was started by this session.
/// No forced tree termination is used if this graceful request fails.
pub async fn stop_emulator(config: &AndroidConfig, serial: &str) -> Result<(), String> {
    require_complete(config)?;
    emulator_port(serial)?;
    select_device(&list_devices(config).await?, serial)?;
    let output = target_tool(config, serial, &["emu", "kill"], TOOL_TIMEOUT).await?;
    if !output.success {
        return Err("Эмулятор не подтвердил корректное завершение. Принудительная остановка не выполнялась.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with(config: serde_json::Value) -> Profile {
        serde_json::from_value(serde_json::json!({
            "id": "android", "name": "Android", "subtitle": "Sample application",
            "kind": "android", "path": "", "accent": "#123456", "cover": "android",
            "favorite": false, "tags": [], "config": config,
        }))
        .unwrap()
    }

    fn fixture_sdk() -> &'static str {
        r"C:\fixtures\android-sdk"
    }

    #[test]
    fn android_paths_share_strict_windows_lexical_validation_without_io() {
        assert_eq!(
            validate_absolute_path(fixture_sdk(), "Android SDK").unwrap(),
            PathBuf::from(fixture_sdk())
        );
        for invalid in [
            r"C:\fixtures\sdk:payload",
            r"C:\fixtures\sdk*",
            r"C:\fixtures\sdk?",
            r"C:\fixtures\sdk|other",
            r"C:\fixtures\<sdk>",
            r#"C:\fixtures\sdk"other"#,
            "C:\\fixtures\\sdk\n",
            r"\\server\share\sdk",
            r"//server/share/sdk",
            r"C:\fixtures\..\sdk",
            r"C:\fixtures\.\sdk",
            r"C:\fixtures\sdk.",
            "C:\\fixtures\\sdk ",
            r"C:\fixtures\folder.\sdk",
            "C:\\fixtures\\folder \\sdk",
            r"C:relative",
            "relative/sdk",
            "/fixtures/android-sdk",
        ] {
            let error = validate_absolute_path(invalid, "Android SDK").unwrap_err();
            assert!(error.contains("Android SDK"));
            assert!(validate(&profile_with(serde_json::json!({"sdkRoot": invalid}))).is_err());
        }
    }

    #[tokio::test]
    async fn invalid_sdk_paths_are_rejected_before_android_tool_resolution() {
        let before = crate::runtime::native_effect_attempts();
        for invalid in [
            r"C:\fixtures\sdk:payload",
            r"C:\fixtures\sdk*",
            "relative/sdk",
        ] {
            let mut config = default_config();
            config.sdk_root = Some(invalid.into());
            config.serial = Some("emulator-5554".into());
            assert!(issues(&config).is_empty());
            assert!(require_complete(&config).is_err());
            assert!(adb_path(&config).is_err());
            assert!(emulator_env(&config).is_err());
            assert!(list_devices(&config).await.is_err());
        }
        assert_eq!(crate::runtime::native_effect_attempts(), before);
    }

    #[test]
    fn setup_accepts_missing_fields_and_never_discovers_a_machine_sdk() {
        let config = validate(&profile_with(serde_json::json!({}))).unwrap();
        assert_eq!(issues(&config).len(), 4);
        assert!(config.sdk_root.is_none());
        assert!(sdk_root(&config).is_err());
        assert!(adb_path(&config).is_err());
    }

    #[test]
    fn setup_normalizes_blank_options_to_absence() {
        let config = validate(&profile_with(serde_json::json!({
            "sdkRoot": " \t", "avdHome": " ", "androidUserHome": "\n",
            "serial": "  ", "avd": " ", "apkPath": " ",
            "packageName": "\t", "activity": " ",
        })))
        .unwrap();
        assert!(config.sdk_root.is_none());
        assert!(config.avd_home.is_none());
        assert!(config.android_user_home.is_none());
        assert!(config.serial.is_none());
        assert!(config.avd.is_none());
        assert!(config.apk_path.is_none());
        assert!(config.package_name.is_empty());
        assert!(config.activity.is_empty());
        assert_eq!(issues(&config).len(), 4);
    }

    #[test]
    fn setup_validates_present_fields_independently_and_denies_unknown_fields() {
        for config in [
            serde_json::json!({"packageName": "com.example;reboot"}),
            serde_json::json!({"activity": ".MainActivity --user 0"}),
            serde_json::json!({"sdkRoot": "relative/sdk"}),
            serde_json::json!({"avd": "Sample_Device;reboot"}),
            serde_json::json!({"serial": "serial with spaces"}),
            serde_json::json!({"command": "ignored"}),
        ] {
            assert!(validate(&profile_with(config)).is_err());
        }
    }

    #[test]
    fn explicit_serial_is_complete_without_an_avd_or_filesystem_probe() {
        let config = validate(&profile_with(serde_json::json!({
            "sdkRoot": fixture_sdk(), "packageName": "com.example.app",
            "activity": ".MainActivity", "serial": "emulator-5554",
        })))
        .unwrap();
        assert!(issues(&config).is_empty());
        assert!(config.avd.is_none());
        assert_eq!(sdk_root(&config).unwrap(), PathBuf::from(fixture_sdk()));
    }

    #[tokio::test]
    async fn incomplete_setup_guards_all_actions_before_tools_or_files() {
        let config = validate(&profile_with(serde_json::json!({}))).unwrap();
        assert!(list_devices(&config).await.is_err());
        assert!(
            wait_boot(&config, "emulator-5554", Duration::from_millis(1))
                .await
                .is_err()
        );
        assert!(app_pid(&config, "emulator-5554").await.is_err());
        assert!(
            wait_app_pid(&config, "emulator-5554", Duration::from_millis(1))
                .await
                .is_err()
        );
        assert!(launch(&config, "emulator-5554").await.is_err());
        assert!(force_stop(&config, "emulator-5554").await.is_err());
        assert!(install(&config, "emulator-5554").await.is_err());
        assert!(emulator_command(&config).is_err());
        assert!(emulator_env(&config).is_err());
        assert!(avd_name(&config, "emulator-5554").await.is_err());
        assert!(stop_emulator(&config, "emulator-5554").await.is_err());
    }

    #[test]
    fn parse_devices_preserves_explicit_identity_and_state() {
        let devices = parse_devices("List of devices attached\n\nemulator-5554\tdevice product:sdk model:Sample_Device device:emu transport_id:1\nABC123\tunauthorized usb:1-1\nemulator-5556\toffline\n");
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].label, "Sample Device");
        assert_eq!(devices[1].state, "unauthorized");
        assert_eq!(devices[2].state, "offline");
    }

    #[test]
    fn wrong_serial_never_falls_back_to_only_connected_device() {
        let devices = parse_devices("List of devices attached\nemulator-5554 device\n");
        assert!(select_device(&devices, "emulator-5556").is_err());
        assert!(select_device(&devices, "emulator-5554").is_ok());
    }

    #[test]
    fn offline_or_unauthorized_device_is_rejected() {
        let devices = parse_devices("List of devices attached\nABC unauthorized\nXYZ offline\n");
        assert!(select_device(&devices, "ABC")
            .unwrap_err()
            .contains("авторизовано"));
        assert!(select_device(&devices, "XYZ")
            .unwrap_err()
            .contains("offline"));
    }

    #[test]
    fn duplicate_serial_is_ambiguous() {
        let devices = parse_devices("ABC device\nABC device\n");
        assert!(select_device(&devices, "ABC").is_err());
    }

    #[test]
    fn rejects_shell_fragments_and_invalid_components() {
        for value in [
            "",
            "-d",
            "serial;adb",
            "serial\nother",
            "serial with spaces",
        ] {
            assert!(validate_serial(value).is_err(), "{value:?}");
        }
        assert!(validate_component("com.example.app", ".MainActivity").is_ok());
        assert!(validate_component("com.example.app", "com.example.app.MainActivity").is_ok());
        assert!(validate_component("com.example.app;reboot", ".MainActivity").is_err());
        assert!(validate_component("com.example.app", ".MainActivity --user 0").is_err());
        assert!(validate_component("com..example", ".MainActivity").is_err());
    }

    #[test]
    fn emulator_port_is_exact_and_never_guessed() {
        assert_eq!(emulator_port("emulator-5554").unwrap(), 5554);
        for value in [
            "ABC123",
            "emulator-5555",
            "emulator-5684",
            "emulator-05554",
            "emulator-1",
        ] {
            assert!(emulator_port(value).is_err(), "{value}");
        }
    }

    #[test]
    fn avd_name_requires_identity_instead_of_ok_or_error() {
        assert_eq!(
            parse_avd_name("\r\nSample_Device\r\nOK\r\n").unwrap(),
            "Sample_Device"
        );
        assert!(parse_avd_name("\nOK\n").is_err());
        assert!(parse_avd_name("KO: unknown command\n").is_err());
    }

    #[test]
    fn emulator_environment_is_a_fixed_whitelist() {
        let mut config = default_config();
        config.avd = Some("Sample_Device".into());
        config.sdk_root = Some(fixture_sdk().into());
        config.avd_home = Some(r"C:\fixtures\avd".into());
        config.android_user_home = Some(r"C:\fixtures\android-user".into());
        let env = emulator_env(&config).unwrap();
        let keys: Vec<_> = env.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "ANDROID_SDK_ROOT",
                "ANDROID_HOME",
                "ANDROID_AVD_HOME",
                "ANDROID_USER_HOME"
            ]
        );
        config.avd_home = Some("../outside".into());
        assert!(emulator_env(&config).is_err());
    }

    #[test]
    fn pid_absence_never_conflates_transport_failure_or_empty_success() {
        assert_eq!(parse_app_pid(false, Some(1), "", "").unwrap(), None);
        assert_eq!(
            parse_app_pid(true, Some(0), "1234\n", "").unwrap(),
            Some(1234)
        );
        assert!(parse_app_pid(false, Some(1), "", "device offline").is_err());
        assert!(parse_app_pid(false, Some(127), "", "").is_err());
        assert!(parse_app_pid(false, None, "", "").is_err());
        assert!(parse_app_pid(true, Some(0), "", "").is_err());
        assert!(parse_app_pid(true, Some(0), "1234 5678", "").is_err());
    }

    #[tokio::test]
    async fn wait_pid_accepts_delayed_process_creation() {
        let mut responses = std::collections::VecDeque::from([Ok(None), Ok(None), Ok(Some(4321))]);
        let mut calls = 0;
        let pid = poll_app_pid(Duration::from_secs(1), Duration::from_millis(1), || {
            calls += 1;
            let response = responses.pop_front().expect("unexpected extra probe");
            async move { response }
        })
        .await
        .unwrap();
        assert_eq!(pid, 4321);
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn wait_pid_times_out_when_process_never_appears() {
        let mut calls = 0;
        let error = poll_app_pid(Duration::from_millis(15), Duration::from_millis(1), || {
            calls += 1;
            async { Ok(None) }
        })
        .await
        .unwrap_err();
        assert!(calls > 0);
        assert!(error.contains("Истекло время"));
    }

    #[tokio::test]
    async fn wait_pid_preserves_transport_error_without_retry() {
        let mut calls = 0;
        let error = poll_app_pid(Duration::from_secs(1), Duration::from_millis(250), || {
            calls += 1;
            async { Err("transport unavailable".to_string()) }
        })
        .await
        .unwrap_err();
        assert_eq!(error, "transport unavailable");
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn wait_pid_bounds_each_pending_probe_by_remaining_budget() {
        let error = poll_app_pid(Duration::from_millis(10), Duration::from_millis(1), || {
            std::future::pending::<Result<Option<u32>, String>>()
        })
        .await
        .unwrap_err();
        assert!(error.contains("Истекло время"));
    }
}
