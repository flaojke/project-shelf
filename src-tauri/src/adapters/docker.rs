//! Narrow Docker Compose adapter. Never reads container Env or emits tool output.
//! All mutations address one service in one registered compose file.

use crate::{
    models::Profile,
    runtime::{run_tool, ToolOutput},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

const TOOL_TIMEOUT: Duration = Duration::from_secs(20);
const START_TIMEOUT: Duration = Duration::from_secs(90);
const STOP_TIMEOUT: Duration = Duration::from_secs(25);

// This Go template intentionally exposes no environment variables, mounts,
// commands, networks, or other potentially sensitive docker-inspect content.
const IDENTITY_TEMPLATE: &str = r#"{"id":{{json .Id}},"running":{{json .State.Running}},"restarting":{{json .State.Restarting}},"paused":{{json .State.Paused}},"service":{{json (index .Config.Labels "com.docker.compose.service")}},"configFiles":{{json (index .Config.Labels "com.docker.compose.project.config_files")}}}"#;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DockerConfig {
    pub compose_file: String,
    pub service: String,
    pub docker_path: Option<String>,
    pub health_message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceState {
    /// Any matching container is running or restarting. Such a service is
    /// external on application restart and must never be adopted implicitly.
    pub running: bool,
    /// Full, untruncated IDs, including stopped containers of this service.
    pub container_ids: Vec<String>,
}

/// Read-only observations only. Availability never grants process ownership.
#[derive(Clone, Debug, Default)]
pub struct DockerDiagnosticsObservation {
    pub cli_available: bool,
    pub engine_ready: Option<bool>,
    pub container_ids: Vec<String>,
    pub running: Option<bool>,
    pub identity_verified: bool,
    pub dependency_message: String,
    pub process_message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerIdentity {
    id: String,
    running: bool,
    restarting: bool,
    paused: bool,
    service: Option<String>,
    config_files: Option<String>,
}

pub fn validate(profile: &Profile) -> Result<DockerConfig, String> {
    let config: DockerConfig = serde_json::from_value(profile.config.clone())
        .map_err(|_| "Некорректный профиль Docker: проверьте composeFile и service.".to_string())?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &DockerConfig) -> Result<(), String> {
    let path = &config.compose_file;
    if !path.is_empty()
        && (path.len() > 32767 || !is_absolute_windows_path(path) || has_unsafe_path_part(path))
    {
        return Err("Compose file должен иметь абсолютный локальный путь без переходов к родительским папкам.".into());
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("");
    if !path.is_empty()
        && !extension.eq_ignore_ascii_case("yaml")
        && !extension.eq_ignore_ascii_case("yml")
    {
        return Err("Выберите существующий compose-файл .yaml или .yml.".into());
    }
    if !config.service.is_empty()
        && (config.service.len() > 120
            || !config.service.as_bytes()[0].is_ascii_alphanumeric()
            || !config
                .service
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)))
    {
        return Err("Некорректное имя Docker Compose service.".into());
    }
    if let Some(executable) = &config.docker_path {
        let basename = executable.rsplit(['\\', '/']).next().unwrap_or("");
        if !executable.is_empty()
            && executable != "docker"
            && executable != "docker.exe"
            && !(executable.len() <= 32767
                && is_absolute_windows_path(executable)
                && !has_unsafe_path_part(executable)
                && basename.eq_ignore_ascii_case("docker.exe"))
        {
            return Err("Разрешён только docker из PATH либо абсолютный путь к docker.exe.".into());
        }
    }
    if config.health_message.len() > 128 || config.health_message.chars().any(char::is_control) {
        return Err("Некорректный ожидаемый ответ готовности Docker-сервиса.".into());
    }
    Ok(())
}

/// Pure setup diagnostics. No filesystem access or tool probing is performed.
pub fn issues(config: &DockerConfig) -> Vec<String> {
    let mut missing = Vec::new();
    if config.compose_file.is_empty() {
        missing.push("Укажите compose file.".into());
    }
    if config.service.is_empty() {
        missing.push("Укажите Docker Compose service.".into());
    }
    if config.health_message.trim().is_empty() {
        missing.push("Укажите ожидаемое сообщение /health.".into());
    }
    missing
}

/// All executable paths must pass this guard before even filesystem metadata
/// or Docker is queried. Safe partial profiles are storage-only until complete.
fn require_complete(config: &DockerConfig) -> Result<(), String> {
    validate_config(config)?;
    let missing = issues(config);
    if !missing.is_empty() {
        return Err(format!("Сначала настройте Docker: {}", missing.join(" ")));
    }
    Ok(())
}

fn is_absolute_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn has_unsafe_path_part(value: &str) -> bool {
    value
        .chars()
        .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '*' | '?'))
        || value[2..].contains(':')
        || value
            .split(['\\', '/'])
            .any(|s| s == ".." || s == "." || s.ends_with(' ') || s.ends_with('.'))
}

fn executable(config: &DockerConfig) -> &str {
    config
        .docker_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .unwrap_or("docker")
}

/// Keep dependency diagnostics fail-closed if a later edit supplies a lifecycle
/// command. Service identity queries use inspect()'s separate fixed commands.
fn read_only_diagnostic_args(args: &[String]) -> bool {
    (args.len() == 1 && args[0] == "--version")
        || (args.len() == 3
            && args[0] == "info"
            && args[1] == "--format"
            && args[2] == "{{.OSType}}")
}

async fn diagnostic_query(config: &DockerConfig, args: &[String]) -> Result<ToolOutput, String> {
    if !read_only_diagnostic_args(args) {
        return Err("Диагностика разрешает только чтение версии и состояния Docker Engine.".into());
    }
    run_tool(executable(config), args, None, Duration::from_secs(8)).await
}

/// Queries only CLI version, engine information and exact registered service
/// identity. In particular, this path must never call ensure_engine(): even a
/// stopped Docker Desktop stays stopped when the user opens diagnostics.
pub async fn diagnose(profile: &Profile) -> DockerDiagnosticsObservation {
    let mut observation = DockerDiagnosticsObservation {
        dependency_message: "Docker CLI и Engine ещё не проверены.".into(),
        process_message: "Контейнеры не проверены; владение не назначалось.".into(),
        ..DockerDiagnosticsObservation::default()
    };
    if profile.kind != crate::models::ProjectKind::Docker {
        observation.dependency_message =
            "Диагностика Docker доступна только Docker-профилю.".into();
        return observation;
    }
    // Partial public profiles remain storage-only, including a missing URL.
    if let Err(message) = crate::validate::require_complete(profile) {
        observation.dependency_message = message;
        return observation;
    }
    let config = match validate(profile) {
        Ok(config) => config,
        Err(message) => {
            observation.dependency_message = message;
            return observation;
        }
    };
    if let Err(message) = require_complete(&config) {
        observation.dependency_message = message;
        return observation;
    }
    let version = diagnostic_query(&config, &["--version".into()]).await;
    match version {
        Ok(output) if output.success && output.stdout.trim().starts_with("Docker version ") => {
            observation.cli_available = true;
        }
        _ => {
            observation.dependency_message = "Docker CLI недоступен или проверка --version не подтвердилась. Engine и контейнеры не проверялись.".into();
            return observation;
        }
    }
    let engine = diagnostic_query(
        &config,
        &["info".into(), "--format".into(), "{{.OSType}}".into()],
    )
    .await;
    match engine {
        Ok(output) if output.success && output.stdout.trim() == "linux" => {
            observation.engine_ready = Some(true);
            observation.dependency_message =
                "Docker CLI и Linux Engine доступны; диагностика ничего не запускала.".into();
        }
        Ok(output) if output.success => {
            observation.engine_ready = Some(false);
            observation.dependency_message = "Docker CLI доступен, но подходящий Linux Engine не подтверждён. Контекст и Docker Desktop не изменялись.".into();
            return observation;
        }
        _ => {
            observation.engine_ready = Some(false);
            observation.dependency_message = "Docker CLI доступен, но Engine не ответил успешно. Возможны остановленный Engine или отсутствие доступа; Docker Desktop не запускался.".into();
            return observation;
        }
    }
    match inspect(&config).await {
        Ok(state) => {
            observation.running = Some(state.running);
            observation.identity_verified = !state.container_ids.is_empty();
            observation.process_message = if state.container_ids.is_empty() {
                "Для точного compose file и сервиса контейнеры не найдены. Владение не назначалось."
                    .into()
            } else if state.running {
                "Контейнеры сервиса работают; точные compose/service labels подтверждены. Принадлежность текущему сеансу проверяется отдельно.".into()
            } else {
                "Контейнеры сервиса остановлены; точные compose/service labels подтверждены.".into()
            };
            observation.container_ids = state.container_ids;
        }
        Err(message) => observation.process_message = message,
    }
    observation
}

/// Explicit Start may bring up Docker Desktop, but never installs it or changes
/// contexts. Queries, Open and Stop do not call this function.
pub async fn ensure_engine(config: &DockerConfig) -> Result<(), String> {
    require_complete(config)?;
    async fn engine(config: &DockerConfig) -> Result<bool, String> {
        let args = vec!["info".into(), "--format".into(), "{{.OSType}}".into()];
        match run_tool(executable(config),&args,None,Duration::from_secs(8)).await {
            Ok(output) if output.success && output.stdout.trim()=="linux"=>Ok(true),
            Ok(output) if output.success=>Err("Для Docker-профиля требуется Linux Engine. Выберите его в Docker Desktop; контекст автоматически не меняется.".into()),
            _=>Ok(false),
        }
    }
    if engine(config).await? {
        return Ok(());
    }
    let args = vec!["desktop".into(), "start".into(), "--detach".into()];
    let output=run_tool(executable(config),&args,None,Duration::from_secs(25)).await.map_err(|_|"Docker Desktop недоступен. Установите или откройте его вручную; автоматической установки нет.".to_string())?;
    if !output.success {
        return Err("Docker Desktop не подтвердил запуск. Откройте его и проверьте Linux Engine; данные не изменялись.".into());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        if engine(config).await? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "Docker Engine не стал готов за 180 секунд. Контейнер не запускался.".into(),
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Canonicalize only the registered compose file, without reading its content.
fn compose_file(config: &DockerConfig) -> Result<PathBuf, String> {
    require_complete(config)?;
    let path = Path::new(&config.compose_file);
    if !path.is_file() {
        return Err("Compose file не найден. Проверьте путь в настройках проекта.".into());
    }
    std::fs::canonicalize(path).map_err(|_| "Не удалось подтвердить путь к compose file.".into())
}

fn command_path(path: &Path) -> Result<String, String> {
    // Docker Desktop expects a normal Windows drive path, not an extended path.
    let text = path
        .to_str()
        .ok_or("Путь compose file не является корректной строкой.")?;
    Ok(text.strip_prefix(r"\\?\").unwrap_or(text).to_string())
}

fn compose_args(file: &str, tail: &[&str]) -> Vec<String> {
    let mut args = vec!["compose".into(), "--file".into(), file.into()];
    args.extend(tail.iter().map(|v| (*v).to_string()));
    args
}

fn parse_json_records(text: &str) -> Result<Vec<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return match value {
            Value::Array(items) => Ok(items),
            Value::Object(_) => Ok(vec![value]),
            _ => Err("Docker вернул неожиданный формат списка контейнеров.".into()),
        };
    }
    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .map_err(|_| "Не удалось прочитать список контейнеров Docker.".to_string())
        })
        .collect()
}

fn parse_compose_ids(text: &str, service: &str) -> Result<Vec<String>, String> {
    let mut ids = BTreeSet::new();
    for row in parse_json_records(text)? {
        let row_service = row
            .get("Service")
            .and_then(Value::as_str)
            .ok_or("Docker Compose не сообщил service контейнера.")?;
        if row_service != service {
            return Err(
                "Docker Compose вернул контейнер другого сервиса; операция отменена.".into(),
            );
        }
        let id = row
            .get("ID")
            .and_then(Value::as_str)
            .ok_or("Docker Compose не сообщил полный ID контейнера.")?;
        if !valid_container_id(id) {
            return Err("Docker Compose не сообщил полный ID контейнера; безопасное владение не подтверждено.".into());
        }
        if !ids.insert(id.to_string()) {
            return Err("Docker Compose вернул дублирующиеся ID контейнеров.".into());
        }
    }
    Ok(ids.into_iter().collect())
}

fn valid_container_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Match only a nonempty, unambiguous set of full saved IDs. The caller must
/// separately verify compose/service labels before treating them as owned.
pub fn matching_container_ids(expected: &[String], actual: &[String]) -> bool {
    let expected_set: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let actual_set: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    !expected_set.is_empty()
        && expected_set.len() == expected.len()
        && actual_set.len() == actual.len()
        && expected_set.iter().all(|id| valid_container_id(id))
        && actual_set.iter().all(|id| valid_container_id(id))
        && expected_set == actual_set
}

fn normalized_file_identity(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_lowercase()
}

fn verify_identities(
    text: &str,
    requested_ids: &[String],
    config: &DockerConfig,
    file: &str,
) -> Result<ServiceState, String> {
    let requested: BTreeSet<&str> = requested_ids.iter().map(String::as_str).collect();
    let mut actual = BTreeSet::new();
    let mut running = false;
    for value in parse_json_records(text)? {
        let identity: ContainerIdentity = serde_json::from_value(value)
            .map_err(|_| "Не удалось подтвердить идентичность Docker-контейнера.".to_string())?;
        if !valid_container_id(&identity.id)
            || !requested.contains(identity.id.as_str())
            || !actual.insert(identity.id.clone())
        {
            return Err("Идентификаторы контейнеров Docker не совпали; операция отменена.".into());
        }
        if identity.service.as_deref() != Some(config.service.as_str()) {
            return Err("Контейнер принадлежит другому Docker Compose service.".into());
        }
        // Multiple compose files are deliberately unsupported: this launcher
        // must not affect a stack assembled with additional override files.
        let files = identity
            .config_files
            .as_deref()
            .ok_or("У контейнера нет Compose identity labels.")?;
        if files.contains(',') || normalized_file_identity(files) != normalized_file_identity(file)
        {
            return Err("Контейнер создан другим compose-файлом; владение не подтверждено.".into());
        }
        if identity.paused {
            return Err(
                "Docker-сервис приостановлен вне Project Shelf. Возобновите его в Docker Desktop."
                    .into(),
            );
        }
        running |= identity.running || identity.restarting;
    }
    if actual.len() != requested.len() {
        return Err(
            "Состав контейнеров изменился во время проверки; повторите обновление статуса.".into(),
        );
    }
    Ok(ServiceState {
        running,
        container_ids: actual.into_iter().collect(),
    })
}

/// Read-only identity check. The filtered inspect template cannot expose Env.
pub async fn inspect(config: &DockerConfig) -> Result<ServiceState, String> {
    let canonical = compose_file(config)?;
    let file = command_path(&canonical)?;
    let args = compose_args(
        &file,
        &[
            "ps",
            "--all",
            "--no-trunc",
            "--format",
            "json",
            &config.service,
        ],
    );
    let output = run_tool(executable(config), &args, canonical.parent(), TOOL_TIMEOUT)
        .await
        .map_err(|_| {
            "Docker недоступен. Проверьте установку и запуск Docker Desktop.".to_string()
        })?;
    if !output.success {
        return Err(
            "Не удалось проверить Docker Compose service. Проверьте Docker Desktop и compose file."
                .into(),
        );
    }
    let ids = parse_compose_ids(&output.stdout, &config.service)?;
    if ids.is_empty() {
        return Ok(ServiceState::default());
    }
    let mut args = vec![
        "inspect".into(),
        "--type".into(),
        "container".into(),
        "--format".into(),
        IDENTITY_TEMPLATE.into(),
    ];
    args.extend(ids.iter().cloned());
    let output = run_tool(executable(config), &args, canonical.parent(), TOOL_TIMEOUT)
        .await
        .map_err(|_| "Не удалось подтвердить идентичность Docker-контейнеров.".to_string())?;
    if !output.success {
        return Err(
            "Контейнер изменился или недоступен; безопасное владение не подтверждено.".into(),
        );
    }
    verify_identities(&output.stdout, &ids, config, &file)
}

/// Starts only this service, never its dependencies. The supervisor must first
/// authorize live actions and record the preflight state; success alone does
/// not establish ownership. Capture and verify inspect() IDs after this call.
pub async fn start(config: &DockerConfig) -> Result<(), String> {
    if inspect(config).await?.running {
        return Err("Docker-сервис уже запущен. Используйте существующий экземпляр без присвоения владения.".into());
    }
    let canonical = compose_file(config)?;
    let file = command_path(&canonical)?;
    // Starting a project is not permission to build or download its runtime.
    // Missing images are an explicit setup error, never an implicit install.
    let args = compose_args(
        &file,
        &[
            "up",
            "--detach",
            "--no-deps",
            "--no-build",
            "--pull",
            "never",
            &config.service,
        ],
    );
    let output = run_tool(executable(config), &args, canonical.parent(), START_TIMEOUT)
        .await
        .map_err(|_| "Не удалось выполнить запуск Docker-сервиса.".to_string())?;
    if !output.success {
        return Err(
            "Docker не смог запустить сервис. Проверьте Docker Desktop и доступность образа."
                .into(),
        );
    }
    Ok(())
}

fn confirm_owned(current: &ServiceState, owned_ids: &[String]) -> Result<(), String> {
    if !matching_container_ids(owned_ids, &current.container_ids) {
        return Err(
            "Контейнеры не принадлежат текущему запуску Project Shelf; остановка запрещена.".into(),
        );
    }
    Ok(())
}

fn stop_args(owned_ids: &[String]) -> Result<Vec<String>, String> {
    let ids: BTreeSet<&str> = owned_ids.iter().map(String::as_str).collect();
    if ids.is_empty() || ids.iter().any(|id| !valid_container_id(id)) {
        return Err(
            "Нет подтверждённых полных ID собственных контейнеров; остановка запрещена.".into(),
        );
    }
    let mut args = vec!["stop".into(), "--time".into(), "20".into()];
    args.extend(ids.into_iter().map(str::to_string));
    Ok(args)
}

/// Leaves all volumes, images, networks, other services and Docker Desktop
/// untouched. Rechecks IDs and compose/service labels immediately before
/// stopping, then targets the saved full IDs rather than resolving the service
/// name again. A concurrent replacement is therefore never stopped.
pub async fn stop(config: &DockerConfig, owned_ids: &[String]) -> Result<(), String> {
    let current = inspect(config).await?;
    confirm_owned(&current, owned_ids)?;
    if !current.running {
        return Ok(());
    }
    let canonical = compose_file(config)?;
    let args = stop_args(owned_ids)?;
    let output = run_tool(executable(config), &args, canonical.parent(), STOP_TIMEOUT)
        .await
        .map_err(|_| "Не удалось остановить собственный Docker-сервис.".to_string())?;
    if !output.success {
        return Err(
            "Docker не подтвердил остановку сервиса. Обновите статус в Docker Desktop.".into(),
        );
    }
    let after = inspect(config).await?;
    // An external replacement is reported as unowned. Never follow its new
    // IDs with another stop command.
    confirm_owned(&after, owned_ids)?;
    if after.running {
        return Err("Docker-сервис продолжает работать после команды остановки.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: char) -> String {
        value.to_string().repeat(64)
    }
    fn config() -> DockerConfig {
        DockerConfig {
            compose_file: r"C:\fixtures\containers\compose.yaml".into(),
            service: "sample_service".into(),
            docker_path: None,
            health_message: "ready".into(),
        }
    }
    fn identity(id: &str, file: &str, service: &str) -> String {
        serde_json::json!({"id":id,"running":true,"restarting":false,"paused":false,"service":service,"configFiles":file}).to_string()
    }
    #[test]
    fn parse_compose_array_and_ndjson() {
        let a = serde_json::json!({"ID":id('a'),"Service":"sample_service"});
        let b = serde_json::json!({"ID":id('b'),"Service":"sample_service"});
        assert_eq!(
            parse_compose_ids(
                &serde_json::json!([a.clone(), b.clone()]).to_string(),
                "sample_service"
            )
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            parse_compose_ids(&format!("{a}\n{b}\n"), "sample_service")
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn incomplete_id_and_other_service_are_rejected() {
        assert!(parse_compose_ids(
            r#"{"ID":"deadbeef","Service":"sample_service"}"#,
            "sample_service"
        )
        .is_err());
        assert!(parse_compose_ids(
            &serde_json::json!({"ID":id('a'),"Service":"db"}).to_string(),
            "sample_service"
        )
        .is_err());
    }
    #[test]
    fn labels_must_match_one_exact_compose_file_and_service() {
        let c = config();
        let owned = vec![id('a')];
        assert!(verify_identities(
            &identity(&owned[0], &c.compose_file, "sample_service"),
            &owned,
            &c,
            &c.compose_file
        )
        .is_ok());
        assert!(verify_identities(
            &identity(
                &owned[0],
                r"C:\fixtures\other\compose.yaml",
                "sample_service"
            ),
            &owned,
            &c,
            &c.compose_file
        )
        .is_err());
        assert!(verify_identities(
            &identity(
                &owned[0],
                &format!("{},override.yaml", c.compose_file),
                "sample_service"
            ),
            &owned,
            &c,
            &c.compose_file
        )
        .is_err());
        assert!(verify_identities(
            &identity(&owned[0], &c.compose_file, "db"),
            &owned,
            &c,
            &c.compose_file
        )
        .is_err());
    }
    #[test]
    fn external_or_replaced_containers_cannot_be_stopped() {
        let state = ServiceState {
            running: true,
            container_ids: vec![id('a')],
        };
        assert!(confirm_owned(&state, &[]).is_err());
        assert!(confirm_owned(&state, &[id('b')]).is_err());
        assert!(confirm_owned(&state, &[id('a')]).is_ok());
    }
    #[test]
    fn matching_ids_accepts_reordering_but_rejects_replacements_and_ambiguous_sets() {
        assert!(matching_container_ids(
            &[id('a'), id('b')],
            &[id('b'), id('a')]
        ));
        for (saved, current) in [
            (vec![], vec![]),
            (vec![id('a')], vec![]),
            (vec![id('a')], vec![id('b')]),
            (vec![id('a'), id('a')], vec![id('a')]),
            (vec![id('a')], vec![id('a'), id('a')]),
            (vec!["deadbeef".into()], vec!["deadbeef".into()]),
            (vec!["g".repeat(64)], vec!["g".repeat(64)]),
            (vec![id('a')], vec![format!("{}\n", id('a'))]),
        ] {
            assert!(!matching_container_ids(&saved, &current));
        }
    }
    #[test]
    fn diagnostics_dependency_queries_cannot_start_or_mutate_docker() {
        let strings = |args: &[&str]| {
            args.iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        };
        assert!(read_only_diagnostic_args(&strings(&["--version"])));
        assert!(read_only_diagnostic_args(&strings(&[
            "info",
            "--format",
            "{{.OSType}}"
        ])));
        for args in [
            vec!["desktop", "start", "--detach"],
            vec![
                "compose",
                "--file",
                "compose.yaml",
                "up",
                "--detach",
                "sample_service",
            ],
            vec!["pull", "sample_image"],
            vec!["stop", "--time", "20", "saved_container_id"],
            vec!["info"],
            vec!["info", "--format", "{{json .}}"],
            vec!["--version", "extra"],
        ] {
            assert!(!read_only_diagnostic_args(&strings(&args)));
        }
    }
    #[test]
    fn no_arbitrary_executable_service_or_relative_path() {
        let mut c = config();
        assert!(validate_config(&c).is_ok());
        c.docker_path = Some("powershell.exe".into());
        assert!(validate_config(&c).is_err());
        c = config();
        c.service = "sample_service --all".into();
        assert!(validate_config(&c).is_err());
        c = config();
        c.compose_file = "compose.yaml".into();
        assert!(validate_config(&c).is_err());
        c = config();
        c.compose_file = r"C:\fixtures\..\compose.yaml".into();
        assert!(validate_config(&c).is_err());
    }
    #[test]
    fn safe_partial_configuration_is_storage_only() {
        let empty: DockerConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(validate_config(&empty).is_ok());
        assert_eq!(issues(&empty).len(), 3);
        assert!(require_complete(&empty).is_err());

        let partial: DockerConfig = serde_json::from_value(serde_json::json!({
            "composeFile": r"C:\fixtures\containers\compose.yaml",
            "service": "sample_service",
            "dockerPath": ""
        }))
        .unwrap();
        assert!(validate_config(&partial).is_ok());
        assert_eq!(issues(&partial).len(), 1);
        assert_eq!(executable(&partial), "docker");
        assert!(require_complete(&partial).is_err());
        assert!(issues(&config()).is_empty());
        assert!(require_complete(&config()).is_ok());
    }
    #[test]
    fn missing_fields_do_not_allow_unsafe_nonempty_values_or_unknown_fields() {
        let mut partial = DockerConfig::default();
        partial.compose_file = "relative.yaml".into();
        assert!(validate_config(&partial).is_err());
        partial.compose_file.clear();
        partial.service = "--other-service".into();
        assert!(validate_config(&partial).is_err());
        partial.service.clear();
        partial.docker_path = Some("powershell.exe".into());
        assert!(validate_config(&partial).is_err());
        partial.docker_path = None;
        partial.health_message = "ready\nsecret".into();
        assert!(validate_config(&partial).is_err());
        assert!(serde_json::from_value::<DockerConfig>(serde_json::json!({
            "executable": "docker"
        }))
        .is_err());
        assert!(serde_json::from_value::<DockerConfig>(serde_json::json!({
            "command": "arbitrary command"
        }))
        .is_err());
    }
    #[tokio::test]
    async fn every_tool_entry_rejects_incomplete_setup_before_os_access() {
        let incomplete = DockerConfig::default();
        for error in [
            ensure_engine(&incomplete).await.unwrap_err(),
            inspect(&incomplete).await.unwrap_err(),
            start(&incomplete).await.unwrap_err(),
            stop(&incomplete, &[id('a')]).await.unwrap_err(),
        ] {
            assert!(error.contains("Сначала настройте Docker"));
        }
    }
    #[test]
    fn compose_mutations_are_narrow() {
        let owned = id('a');
        let args = stop_args(&[owned.clone()]).unwrap();
        assert_eq!(
            args,
            vec!["stop".to_string(), "--time".into(), "20".into(), owned]
        );
        assert!(!args
            .iter()
            .any(|arg| ["down", "--volumes", "--remove-orphans"].contains(&arg.as_str())));
        assert!(!IDENTITY_TEMPLATE.contains(".Config.Env"));
    }
    #[test]
    fn stop_targets_saved_full_ids_and_never_service_replacements() {
        let saved = vec![id('a')];
        let args = stop_args(&saved).unwrap();
        let replacement = ServiceState {
            running: true,
            container_ids: vec![id('b')],
        };
        assert!(confirm_owned(&replacement, &saved).is_err());
        assert!(!args.contains(&id('b')));
        assert!(!args
            .iter()
            .any(|arg| arg == "compose" || arg == "sample_service"));
        assert!(stop_args(&[]).is_err());
        assert!(stop_args(&["deadbeef".into()]).is_err());
    }
}
