//! Read-only, timestamped observations. These checks never prepare or launch a project.
use crate::{
    adapters::{powershell, process},
    models::{DiagnosticCheck, DiagnosticState, Profile, ProjectDiagnostics, ProjectKind},
    validate,
};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use DiagnosticState::{Error, Ready, Unknown};

pub fn check(id: &str, state: DiagnosticState, message: impl Into<String>) -> DiagnosticCheck {
    DiagnosticCheck {
        id: id.into(),
        label: match id {
            "dependencies" => "Зависимости",
            "process" => "Процесс",
            "http" => "HTTP",
            "port" => "Порт",
            _ => "Проверка",
        }
        .into(),
        state,
        message: message.into(),
        value: None,
        http_status: None,
    }
}
pub fn unavailable(id: &str, message: &str, state: DiagnosticState) -> ProjectDiagnostics {
    ProjectDiagnostics {
        id: id.into(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        checks: ["dependencies", "process", "http", "port"]
            .iter()
            .map(|key| check(key, state.clone(), message))
            .collect(),
        technologies: Vec::new(),
        can_stop: false,
        can_restart: false,
        stop_reason: Some(message.into()),
        restart_reason: Some(message.into()),
    }
}
pub fn tool_available(executable: &str) -> bool {
    let path = Path::new(executable);
    if path.is_absolute() {
        return path.is_file();
    }
    if !matches!(
        executable,
        "node" | "node.exe" | "powershell.exe" | "docker" | "docker.exe"
    ) {
        return false;
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|root| {
            root.join(executable).is_file()
                || (path.extension().is_none() && root.join(format!("{executable}.exe")).is_file())
        })
    })
}
fn version(root: &Path, package: &str) -> Option<String> {
    // Only known package-version fields are exposed; never scripts, env, or arbitrary metadata.
    let relative = match package {
        "react" => "node_modules/react/package.json",
        "vite" => "node_modules/vite/package.json",
        _ => return None,
    };
    let path = validate::checked_child(root, relative).ok()?;
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let value = json.get("version")?.as_str()?;
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return None;
    }
    Some(value.into())
}
pub struct WebObservation {
    pub dependencies: DiagnosticCheck,
    pub http: DiagnosticCheck,
    pub port: DiagnosticCheck,
    pub technologies: Vec<String>,
    pub identity_verified: bool,
    pub reported_pid: Option<u32>,
    pub port_number: u16,
}
pub async fn web(client: &reqwest::Client, profile: &Profile) -> Result<WebObservation, String> {
    let base = validate::loopback_url(profile.url.as_deref().ok_or("URL не задан.")?)?;
    let port_number = base.port_or_known_default().ok_or("Порт не задан.")?;
    let root = PathBuf::from(&profile.path);
    let mut technologies = Vec::new();
    let mut missing = Vec::new();
    match profile.kind {
        ProjectKind::Process => {
            let config = process::validate(profile)?;
            if !tool_available(
                config
                    .node_path
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or("node"),
            ) {
                missing.push("Node.js не найден в выбранном пути/PATH");
            }
            if !root.is_dir() {
                missing.push("каталог проекта не найден");
            }
            if validate::checked_child(&root, "node_modules/vite/dist/node/index.js").is_err() {
                missing.push("Vite API не найден в node_modules");
            }
            if let Some(v) = version(&root, "react") {
                technologies.push(format!("React {v}"));
            }
            if let Some(v) = version(&root, "vite") {
                technologies.push(format!("Vite {v}"));
            }
            if tool_available(
                config
                    .node_path
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or("node"),
            ) {
                technologies.push("Node.js (файл найден)".into());
            }
        }
        ProjectKind::Powershell => {
            if !tool_available("powershell.exe") {
                missing.push("PowerShell не найден в PATH");
            }
            if validate::checked_child(&root, "start-site.ps1").is_err() {
                missing.push("start-site.ps1 не найден в каталоге проекта");
            }
            technologies.push("PowerShell / HTTP health".into());
        }
        ProjectKind::Docker => {}
        ProjectKind::Android => return Err("Android не использует HTTP-проверку.".into()),
    }
    let dependencies = check(
        "dependencies",
        if missing.is_empty() { Ready } else { Error },
        if missing.is_empty() {
            "Необходимые файлы найдены. Версии отображаются только из установленных package.json."
                .into()
        } else {
            missing.join("; ")
        },
    );
    let open = process::port_open(port_number).await;
    let mut port = check(
        "port",
        if open { Ready } else { Unknown },
        if open {
            "TCP-соединение установлено. Принадлежность проверяется отдельно."
        } else {
            "TCP-соединение не установлено. Сервер не запущен или ещё недоступен."
        },
    );
    port.value = Some(format!("127.0.0.1:{port_number}"));
    let endpoint = match profile.kind {
        ProjectKind::Powershell => base.join("api/health"),
        ProjectKind::Docker => base.join("health"),
        _ => Ok(base.clone()),
    }
    .map_err(|_| "Некорректный health URL.")?;
    let mut identity_verified = false;
    let mut reported_pid = None;
    let mut http = match process::response(client, endpoint.as_str()).await {
        Ok((status, body)) => {
            if status == 200 {
                identity_verified = match profile.kind {
                    ProjectKind::Process => {
                        process::verify_response(&body, &process::validate(profile)?)
                    }
                    ProjectKind::Powershell => {
                        if let Ok(h) = serde_json::from_str::<powershell::Health>(&body) {
                            let matches = powershell::verify_health(
                                &h,
                                &profile.path,
                                &powershell::validate(profile)?,
                            );
                            if matches {
                                reported_pid = Some(h.pid);
                            }
                            matches
                        } else {
                            false
                        }
                    }
                    ProjectKind::Docker => {
                        let c = crate::adapters::docker::validate(profile)?;
                        serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| {
                                v.get("message")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                            })
                            .as_deref()
                            == Some(c.health_message.as_str())
                    }
                    ProjectKind::Android => false,
                };
            }
            let mut result = check(
                "http",
                if identity_verified { Ready } else { Error },
                if identity_verified {
                    "Ответ получен; identity приложения подтверждена."
                } else {
                    "Ответ получен, но status/identity приложения не соответствуют профилю. Ресурс не присваивается."
                },
            );
            result.http_status = Some(status);
            result.value = Some(format!("HTTP {status}"));
            result
        }
        Err(message) => check("http", if open { Error } else { Unknown }, message),
    };
    // Receiving HTTP confirms a listening endpoint even when an earlier TCP attempt raced.
    if http.http_status.is_some() && !open {
        port.state = Ready;
        port.message = "Получен HTTP-ответ от этого endpoint после TCP-проверки.".into();
    }
    if http.http_status.is_some() && !identity_verified {
        port.state = Error;
        port.message = "Порт занят: identity выбранного приложения не подтверждена.".into();
    }
    http.message = http.message.chars().take(600).collect();
    Ok(WebObservation {
        dependencies,
        http,
        port,
        technologies,
        identity_verified,
        reported_pid,
        port_number,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_observation_never_invents_an_http_status_or_actions() {
        let result = unavailable("fixture", "No measurement", Unknown);
        assert!(!result.can_stop && !result.can_restart);
        assert!(result
            .checks
            .iter()
            .all(|c| c.http_status.is_none() && c.state == Unknown));
    }
}
