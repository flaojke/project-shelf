//! Fixed compatible PowerShell site contract; no arbitrary commands.
use crate::{
    adapters::process::{self, Probe},
    models::Profile,
    validate,
};
use serde::Deserialize;
use std::path::PathBuf;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PowershellConfig {
    pub port: u16,
    pub service: String,
    pub allow_stop: bool,
}
impl Default for PowershellConfig {
    fn default() -> Self {
        Self {
            port: 8877,
            service: String::new(),
            allow_stop: false,
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct Health {
    pub service: String,
    pub root: String,
    pub port: u16,
    pub ready: bool,
    pub pid: u32,
}
pub fn validate(p: &Profile) -> Result<PowershellConfig, String> {
    let c: PowershellConfig = serde_json::from_value(p.config.clone())
        .map_err(|_| "Некорректные поля PowerShell-конфигурации.")?;
    if c.port < 1024 || c.allow_stop {
        return Err("Нужен порт 1024–65535; остановка PowerShell-сайта запрещена.".into());
    }
    if c.service.len() > 128 || c.service.chars().any(char::is_control) {
        return Err("Некорректный health service.".into());
    }
    validate::matching_root_url(p.url.as_deref(), c.port)?;
    Ok(c)
}
pub fn issues(c: &PowershellConfig) -> Vec<String> {
    if c.service.trim().is_empty() {
        vec!["Укажите health service PowerShell-сайта.".into()]
    } else {
        Vec::new()
    }
}
pub fn verify_health(h: &Health, root: &str, c: &PowershellConfig) -> bool {
    h.service == c.service
        && h.ready
        && h.port == c.port
        && h.pid > 0
        && validate::normalized_path(&h.root) == validate::normalized_path(root)
}
pub async fn probe(client: &reqwest::Client, p: &Profile, c: &PowershellConfig) -> Probe {
    if !issues(c).is_empty() || p.path.trim().is_empty() {
        return Probe::Conflict("PowerShell-профиль требует настройки.".into());
    }
    if !process::port_open(c.port).await {
        return Probe::Missing;
    }
    let url = format!("http://127.0.0.1:{}/api/health", c.port);
    if let Ok((200, text)) = process::response(client, &url).await {
        if let Ok(h) = serde_json::from_str::<Health>(&text) {
            if verify_health(&h, &p.path, c) {
                return Probe::Ready { pid: Some(h.pid) };
            }
        }
    }
    Probe::Conflict("Порт занят, но health identity не подтверждает нужный PowerShell-сайт. Остановка запрещена.".into())
}
pub fn command(
    p: &Profile,
    c: &PowershellConfig,
) -> Result<(String, Vec<String>, PathBuf), String> {
    if !issues(c).is_empty() || p.path.trim().is_empty() {
        return Err("Сначала завершите настройку PowerShell-профиля.".into());
    }
    let root = validate::absolute_path(&p.path)?;
    let script = validate::plain_local_path(&validate::checked_child(&root, "start-site.ps1")?)?;
    let root = validate::plain_local_path(
        &root
            .canonicalize()
            .map_err(|_| "Каталог сайта не найден.")?,
    )?;
    Ok((
        "powershell.exe".into(),
        launch_args(&script.to_string_lossy(), c.port),
        root,
    ))
}
fn launch_args(script: &str, port: u16) -> Vec<String> {
    vec![
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-File".into(),
        script.into(),
        "-NoBrowser".into(),
        "-Port".into(),
        port.to_string(),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_health_contract() {
        let c = PowershellConfig {
            service: "sample-service".into(),
            ..Default::default()
        };
        let mut h = Health {
            service: c.service.clone(),
            root: r"C:\fixtures\site".into(),
            port: c.port,
            ready: true,
            pid: 42,
        };
        let root = h.root.clone();
        assert!(verify_health(&h, &root, &c));
        h.ready = false;
        assert!(!verify_health(&h, &root, &c));
        h.ready = true;
        h.service = "other-service".into();
        assert!(!verify_health(&h, &root, &c));
        h.service = c.service.clone();
        h.pid = 0;
        assert!(!verify_health(&h, &root, &c));
        h.pid = 42;
        assert!(!verify_health(&h, r"C:\fixtures\other-site", &c));
    }
    #[test]
    fn fixed_script_arguments() {
        let a = launch_args(r"C:\fixtures\site\start-site.ps1", 8877);
        assert!(a.contains(&"-File".to_string()));
        assert!(!a.contains(&"-ExecutionPolicy".to_string()));
        assert!(!a.contains(&"-Command".into()));
    }
    #[tokio::test]
    async fn incomplete_has_no_probe() {
        let p:Profile=serde_json::from_value(serde_json::json!({"id":"powershell","name":"Site","subtitle":"Setup","kind":"powershell","path":"","accent":"#123456","cover":"powershell","tags":[],"config":{}})).unwrap();
        let c = validate(&p).unwrap();
        assert!(!issues(&c).is_empty());
        assert!(command(&p, &c).is_err());
        assert!(matches!(
            probe(&reqwest::Client::new(), &p, &c).await,
            Probe::Conflict(_)
        ));
    }
}
