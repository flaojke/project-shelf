//! Read-only Android diagnostics over the existing ADB smart socket.
//! No ADB client process is started: version mismatch cannot restart the daemon.
//! Only host:devices-l, transport selection, and three fixed read commands exist.
use super::{
    adb_path, parse_devices, sdk_root, select_device, validate, validate_component,
    validate_serial, AndroidConfig, AndroidDevice,
};
use crate::models::Profile;
use serde::Serialize;
use std::time::Duration;

#[cfg(not(test))]
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const MAX_RESPONSE: usize = 64 * 1024;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidDiagnosticsObservation {
    pub sdk_ready: bool,
    pub dependency_message: String,
    pub serial: Option<String>,
    pub boot_completed: Option<bool>,
    pub app_installed: Option<bool>,
    pub app_pid: Option<u32>,
    pub process_message: String,
    pub errors: Vec<String>,
}

pub async fn diagnose(profile: &Profile) -> AndroidDiagnosticsObservation {
    let mut observation = AndroidDiagnosticsObservation {
        dependency_message: "Android SDK ещё не проверен.".into(),
        process_message: "Состояние Android-устройства ещё не проверено.".into(),
        ..Default::default()
    };
    if let Err(error) = crate::validate::require_complete(profile) {
        observation.dependency_message = "Профиль Android требует настройки.".into();
        observation.process_message = "Диагностика устройства не выполнялась.".into();
        note_error(&mut observation, error);
        return observation;
    }
    let config = match validate(profile) {
        Ok(config) => config,
        Err(error) => {
            observation.dependency_message = "Проверьте конфигурацию Android-профиля.".into();
            observation.process_message = "Диагностика устройства не выполнялась.".into();
            note_error(&mut observation, error);
            return observation;
        }
    };
    let root = match sdk_root(&config) {
        Ok(root) => root,
        Err(error) => {
            observation.dependency_message = "Android SDK не настроен.".into();
            observation.process_message = "Диагностика устройства не выполнялась.".into();
            note_error(&mut observation, error);
            return observation;
        }
    };
    if !root.is_dir() {
        observation.dependency_message = "Каталог Android SDK не найден или недоступен.".into();
        observation.process_message = "Диагностика устройства не выполнялась.".into();
        return observation;
    }
    if !root
        .join("platform-tools")
        .join(if cfg!(windows) { "adb.exe" } else { "adb" })
        .is_file()
    {
        observation.dependency_message =
            "Android SDK найден, но platform-tools/adb недоступен.".into();
        observation.process_message = "Диагностика устройства не выполнялась.".into();
        return observation;
    }
    observation.sdk_ready = true;
    let emulator_present = root
        .join("emulator")
        .join(if cfg!(windows) {
            "emulator.exe"
        } else {
            "emulator"
        })
        .is_file();
    let apk_message = match config
        .apk_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(path) if std::path::Path::new(path).is_file() => "APK доступен.",
        Some(_) => "Выбранный APK не найден; установленное приложение можно проверить.",
        None => "APK не выбран; установка не проверяется.",
    };
    observation.dependency_message = format!(
        "Android SDK и adb доступны. {} {}",
        if emulator_present {
            "Файл Android Emulator найден."
        } else {
            "Файл Android Emulator не найден; подключённые устройства можно проверить."
        },
        apk_message
    );
    let setup = setup_issues(&config);
    if !setup.is_empty() {
        observation.process_message =
            "Заполните настройки приложения и целевого устройства.".into();
        for error in setup {
            note_error(&mut observation, error);
        }
        return observation;
    }
    // Reuse adapter completeness/locator validation without executing the file.
    if let Err(error) = adb_path(&config) {
        observation.process_message = "Не удалось подготовить Android-диагностику.".into();
        note_error(&mut observation, error);
        return observation;
    }
    match tokio::time::timeout(
        DIAGNOSTICS_TIMEOUT,
        observe_device(&config, &mut observation),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            observation.process_message = error.clone();
            note_error(&mut observation, error);
        }
        Err(_) => {
            let error = "Истекло общее время Android-диагностики. Непроверенные поля остаются неизвестными.";
            observation.process_message = error.into();
            note_error(&mut observation, error.into());
        }
    }
    observation
}

fn setup_issues(config: &AndroidConfig) -> Vec<String> {
    let mut missing = Vec::new();
    if config.package_name.trim().is_empty() {
        missing.push("Укажите packageName приложения.".into());
    }
    if config.activity.trim().is_empty() {
        missing.push("Укажите Activity приложения.".into());
    }
    if config.serial.as_deref().unwrap_or("").trim().is_empty()
        && config.avd.as_deref().unwrap_or("").trim().is_empty()
    {
        missing.push("Укажите явный serial или имя AVD.".into());
    }
    missing
}

fn note_error(observation: &mut AndroidDiagnosticsObservation, error: String) {
    // Messages are structured; no raw protocol output, paths or environment.
    if observation.errors.len() < 12 && !observation.errors.contains(&error) {
        observation.errors.push(error);
    }
}

async fn observe_device(
    config: &AndroidConfig,
    observation: &mut AndroidDiagnosticsObservation,
) -> Result<(), String> {
    validate_component(&config.package_name, &config.activity)?;
    let text = query(ReadQuery::Devices).await?;
    let devices = parse_device_payload(&text)?;
    if devices.is_empty() {
        observation.process_message = "Подключённые Android-устройства не обнаружены.".into();
        return Ok(());
    }
    let serial = match explicit_target(config.serial.as_deref(), &devices)? {
        Some(serial) => serial,
        None => {
            observation.process_message = if config.serial.as_deref().unwrap_or("").trim().is_empty() {
                "Устройства подключены, но имя AVD безопасным протоколом не подтверждено. Укажите явный serial для проверки приложения."
            } else {
                "Указанное устройство не подключено. Другое устройство не выбиралось."
            }.into();
            return Ok(());
        }
    };
    observation.serial = Some(serial.clone());
    let boot = query(ReadQuery::Boot {
        serial: serial.clone(),
    })
    .await?;
    observation.boot_completed = parse_boot(&boot)?;
    if observation.boot_completed != Some(true) {
        observation.process_message = if observation.boot_completed == Some(false) {
            "Устройство загружается; приложение пока не проверено."
        } else {
            "Загрузка устройства не подтверждена; приложение пока не проверено."
        }
        .into();
        return Ok(());
    }
    let package = query(ReadQuery::Package {
        serial: serial.clone(),
        package: config.package_name.clone(),
    })
    .await?;
    observation.app_installed = parse_installed(&package, &config.package_name)?;
    if observation.app_installed != Some(true) {
        observation.process_message = if observation.app_installed == Some(false) {
            "Приложение не установлено на выбранном устройстве."
        } else {
            "Package Manager не вернул путь приложения; наличие приложения не подтверждено."
        }
        .into();
        return Ok(());
    }
    let process = query(ReadQuery::Pid {
        serial,
        package: config.package_name.clone(),
    })
    .await?;
    observation.app_pid = parse_pid(&process)?;
    observation.process_message = if observation.app_pid.is_some() {
        "Приложение запущено на выбранном устройстве."
    } else {
        "Приложение установлено; запущенный процесс не подтверждён."
    }
    .into();
    Ok(())
}

fn explicit_target(
    serial: Option<&str>,
    devices: &[AndroidDevice],
) -> Result<Option<String>, String> {
    let Some(serial) = serial.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    validate_serial(serial)?;
    if !devices.iter().any(|device| device.serial == serial) {
        return Ok(None);
    }
    select_device(devices, serial)?;
    Ok(Some(serial.into()))
}

enum ReadQuery {
    Devices,
    Boot { serial: String },
    Package { serial: String, package: String },
    Pid { serial: String, package: String },
}

fn request_for(query: &ReadQuery) -> Result<(Option<&str>, String), String> {
    match query {
        ReadQuery::Devices => Ok((None, "host:devices-l".into())),
        ReadQuery::Boot { serial } => {
            validate_serial(serial)?;
            Ok((
                Some(serial.as_str()),
                "shell:getprop sys.boot_completed".into(),
            ))
        }
        ReadQuery::Package { serial, package } | ReadQuery::Pid { serial, package } => {
            validate_serial(serial)?;
            // Validate again at the only shell-string construction boundary.
            validate_component(package, ".MainActivity")?;
            let command = if matches!(query, ReadQuery::Package { .. }) {
                "pm path"
            } else {
                "pidof"
            };
            Ok((Some(serial.as_str()), format!("shell:{command} {package}")))
        }
    }
}

async fn query(query: ReadQuery) -> Result<String, String> {
    crate::runtime::native_effect_boundary()?;
    // Unit tests never access host ports, even if a complete fixture is supplied.
    #[cfg(test)]
    {
        let _ = query;
        return Err("Native Android diagnostics are disabled in unit tests.".into());
    }
    #[cfg(not(test))]
    {
        tokio::task::spawn_blocking(move || query_existing_server(&query))
            .await
            .map_err(|_| "Не удалось выполнить безопасный запрос Android.".to_string())?
    }
}

#[cfg(not(test))]
fn query_existing_server(query: &ReadQuery) -> Result<String, String> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    let (serial, request) = request_for(query)?;
    let deadline = std::time::Instant::now() + QUERY_TIMEOUT;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5037);
    let mut stream = TcpStream::connect_timeout(&address, remaining(deadline)?).map_err(|_| {
        "Существующий ADB daemon недоступен. Запуск daemon не запрашивался.".to_string()
    })?;
    if let Some(serial) = serial {
        send_request(&mut stream, &format!("host:transport:{serial}"), deadline)?;
    }
    send_request(&mut stream, &request, deadline)?;
    let bytes = if matches!(query, ReadQuery::Devices) {
        let mut length = [0_u8; 4];
        read_exact_bounded(&mut stream, &mut length, deadline)?;
        let length = std::str::from_utf8(&length)
            .ok()
            .and_then(|value| usize::from_str_radix(value, 16).ok())
            .filter(|length| *length <= MAX_RESPONSE)
            .ok_or_else(|| "ADB вернул некорректную длину списка устройств.".to_string())?;
        let mut bytes = vec![0; length];
        read_exact_bounded(&mut stream, &mut bytes, deadline)?;
        bytes
    } else {
        read_to_end_bounded(&mut stream, deadline)?
    };
    String::from_utf8(bytes).map_err(|_| "ADB вернул неподдерживаемый формат ответа.".into())
}

#[cfg(not(test))]
fn remaining(deadline: std::time::Instant) -> Result<Duration, String> {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
        Err("Истекло время безопасного запроса Android.".into())
    } else {
        Ok(remaining)
    }
}

#[cfg(not(test))]
fn read_exact_bounded(
    stream: &mut std::net::TcpStream,
    buffer: &mut [u8],
    deadline: std::time::Instant,
) -> Result<(), String> {
    use std::io::Read;
    let mut offset = 0;
    while offset < buffer.len() {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| "Не удалось ограничить время чтения ADB.".to_string())?;
        let read = stream
            .read(&mut buffer[offset..])
            .map_err(|_| "Не удалось прочитать безопасный ответ ADB.".to_string())?;
        if read == 0 {
            return Err("ADB закрыл соединение до полного ответа.".into());
        }
        offset += read;
    }
    Ok(())
}

#[cfg(not(test))]
fn send_request(
    stream: &mut std::net::TcpStream,
    request: &str,
    deadline: std::time::Instant,
) -> Result<(), String> {
    use std::io::Write;
    if request.len() > 1024 || !request.is_ascii() {
        return Err("Некорректный безопасный запрос ADB.".into());
    }
    let packet = format!("{:04X}{request}", request.len()).into_bytes();
    let mut offset = 0;
    while offset < packet.len() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|_| "Не удалось ограничить время записи ADB.".to_string())?;
        let written = stream
            .write(&packet[offset..])
            .map_err(|_| "Не удалось отправить безопасный запрос ADB.".to_string())?;
        if written == 0 {
            return Err("ADB не принял безопасный запрос.".into());
        }
        offset += written;
    }
    let mut status = [0_u8; 4];
    read_exact_bounded(stream, &mut status, deadline)?;
    if &status != b"OKAY" {
        return Err("ADB отклонил безопасный запрос; daemon не перезапускался.".into());
    }
    Ok(())
}

#[cfg(not(test))]
fn read_to_end_bounded(
    stream: &mut std::net::TcpStream,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| "Не удалось ограничить время чтения ADB.".to_string())?;
        let read = stream
            .read(&mut buffer)
            .map_err(|_| "Не удалось прочитать безопасный ответ Android.".to_string())?;
        if read == 0 {
            return Ok(bytes);
        }
        if bytes.len() + read > MAX_RESPONSE {
            return Err("Ответ Android превысил допустимый размер.".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

fn parse_device_payload(text: &str) -> Result<Vec<AndroidDevice>, String> {
    let devices = parse_devices(text);
    if !text.trim().is_empty() && devices.is_empty() {
        return Err("ADB не подтвердил поддерживаемое состояние устройств.".into());
    }
    Ok(devices)
}

fn parse_boot(text: &str) -> Result<Option<bool>, String> {
    match text.trim() {
        "1" => Ok(Some(true)),
        "0" => Ok(Some(false)),
        "" => Ok(None),
        _ => Err("Устройство не подтвердило корректное значение boot_completed.".into()),
    }
}

fn parse_installed(text: &str, package: &str) -> Result<Option<bool>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.lines().all(|line| {
        line.trim()
            .strip_prefix("package:")
            .is_some_and(|path| !path.trim().is_empty())
    }) {
        return Ok(Some(true));
    }
    if text == format!("Error: package {package} not found") {
        return Ok(Some(false));
    }
    Err("Package Manager не подтвердил наличие или отсутствие приложения.".into())
}

fn parse_pid(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mut fields = text.split_whitespace();
    let pid = fields
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .ok_or_else(|| "Android не подтвердил корректный PID приложения.".to_string())?;
    if fields.next().is_some() {
        return Err("Процесс приложения неоднозначен.".into());
    }
    Ok(Some(pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_observations_do_not_claim_missing_or_stopped() {
        let observation = AndroidDiagnosticsObservation::default();
        assert_eq!(observation.boot_completed, None);
        assert_eq!(observation.app_installed, None);
        assert_eq!(observation.app_pid, None);
    }

    #[test]
    fn no_avd_or_serial_is_guessed_from_a_single_connected_device() {
        let devices = parse_devices("emulator-5554 device\n");
        assert_eq!(explicit_target(None, &devices).unwrap(), None);
        assert_eq!(
            explicit_target(Some("emulator-5556"), &devices).unwrap(),
            None
        );
        assert_eq!(
            explicit_target(Some("emulator-5554"), &devices).unwrap(),
            Some("emulator-5554".into())
        );
    }

    #[test]
    fn legacy_shell_empty_is_unknown_and_errors_are_not_false_absence() {
        assert_eq!(parse_boot("1\n").unwrap(), Some(true));
        assert_eq!(parse_boot("0\n").unwrap(), Some(false));
        assert_eq!(parse_boot("").unwrap(), None);
        assert!(parse_boot("permission denied").is_err());
        assert_eq!(
            parse_installed("package:/data/app/example/base.apk\n", "com.example.app").unwrap(),
            Some(true)
        );
        assert_eq!(parse_installed("", "com.example.app").unwrap(), None);
        assert!(parse_installed("device offline", "com.example.app").is_err());
        assert_eq!(parse_pid("1234\n").unwrap(), Some(1234));
        assert_eq!(parse_pid("").unwrap(), None);
        assert!(parse_pid("1234 5678").is_err());
    }

    #[test]
    fn only_validated_fixed_read_commands_can_be_built() {
        let devices_query = ReadQuery::Devices;
        let (serial, request) = request_for(&devices_query).unwrap();
        assert_eq!(serial, None);
        assert_eq!(request, "host:devices-l");
        let query = ReadQuery::Package {
            serial: "emulator-5554".into(),
            package: "com.example.app".into(),
        };
        let (serial, request) = request_for(&query).unwrap();
        assert_eq!(serial, Some("emulator-5554"));
        assert_eq!(request, "shell:pm path com.example.app");
        assert!(request_for(&ReadQuery::Pid {
            serial: "emulator-5554".into(),
            package: "com.example.app;reboot".into()
        })
        .is_err());
    }
}
