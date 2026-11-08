use crate::{models::Profile, runtime, validate};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ProcessConfig {
    pub node_path: Option<String>,
    pub port: u16,
    pub ready_marker: String,
}
impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            node_path: None,
            port: 5180,
            ready_marker: String::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    Missing,
    Ready { pid: Option<u32> },
    Conflict(String),
}
pub fn validate(p: &Profile) -> Result<ProcessConfig, String> {
    let mut c: ProcessConfig = serde_json::from_value(p.config.clone())
        .map_err(|_| "Некорректные поля Vite-конфигурации.")?;
    if c.port < 1024 {
        return Err("Нужен порт 1024–65535.".into());
    }
    if c.ready_marker.len() > 256 || c.ready_marker.chars().any(char::is_control) {
        return Err("Некорректный readyMarker.".into());
    }
    if c.node_path.as_deref().is_some_and(|s| s.trim().is_empty()) {
        c.node_path = None;
    }
    if let Some(n) = &c.node_path {
        validate_node(n)?;
    }
    validate::matching_root_url(p.url.as_deref(), c.port)?;
    Ok(c)
}
pub fn issues(c: &ProcessConfig) -> Vec<String> {
    if c.ready_marker.trim().is_empty() {
        vec!["Укажите readyMarker веб-проекта.".into()]
    } else {
        Vec::new()
    }
}
fn validate_node(v: &str) -> Result<(), String> {
    if matches!(v, "node" | "node.exe") {
        return Ok(());
    }
    let p = validate::absolute_path(v)?;
    if !p
        .file_name()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("node.exe"))
    {
        return Err("Выберите node.exe или Node.js из PATH.".into());
    }
    Ok(())
}
pub fn command(
    p: &Profile,
    c: &ProcessConfig,
    bridge: &Path,
) -> Result<(String, Vec<String>, PathBuf), String> {
    if !issues(c).is_empty() || p.path.trim().is_empty() {
        return Err("Сначала завершите настройку веб-профиля.".into());
    }
    let root = validate::absolute_path(&p.path)?;
    validate::checked_child(&root, "node_modules/vite/dist/node/index.js")?;
    let root = validate::plain_local_path(
        &root
            .canonicalize()
            .map_err(|_| "Каталог проекта не найден.")?,
    )?;
    let bridge = validate::plain_local_path(
        &bridge
            .canonicalize()
            .map_err(|_| "Управляющий bridge не найден в AppData.")?,
    )?;
    let n = c.node_path.as_deref().unwrap_or("node");
    validate_node(n)?;
    if Path::new(n).is_absolute() && !Path::new(n).is_file() {
        return Err("Выбранный node.exe не найден.".into());
    }
    Ok((
        n.into(),
        vec![
            bridge.to_string_lossy().into_owned(),
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "--port".into(),
            c.port.to_string(),
        ],
        root,
    ))
}
pub async fn port_open(port: u16) -> bool {
    if runtime::native_effect_boundary().is_err() {
        return false;
    }
    matches!(
        tokio::time::timeout(
            Duration::from_millis(350),
            tokio::net::TcpStream::connect(("127.0.0.1", port))
        )
        .await,
        Ok(Ok(_))
    )
}
pub async fn response(client: &reqwest::Client, v: &str) -> Result<(u16, String), String> {
    runtime::native_effect_boundary()?;
    validate::loopback_url(v)?;
    let mut r = client
        .get(v)
        .send()
        .await
        .map_err(|_| "Локальный сервер не ответил вовремя.")?;
    let status = r.status().as_u16();
    let mut b = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|_| "Ошибка чтения ответа.")? {
        if b.len() + c.len() > 256 * 1024 {
            return Err("Ответ превышает допустимый размер.".into());
        }
        b.extend_from_slice(&c);
    }
    Ok((status, String::from_utf8_lossy(&b).into_owned()))
}
pub fn verify_response(text: &str, c: &ProcessConfig) -> bool {
    !c.ready_marker.trim().is_empty()
        && text.contains(&c.ready_marker)
        && text.contains("/@vite/client")
}
pub async fn probe(client: &reqwest::Client, p: &Profile, c: &ProcessConfig) -> Probe {
    if !issues(c).is_empty()
        || p.path.trim().is_empty()
        || p.url.as_deref().unwrap_or("").trim().is_empty()
    {
        return Probe::Conflict("Веб-профиль требует настройки.".into());
    }
    if !port_open(c.port).await {
        return Probe::Missing;
    }
    match response(client,p.url.as_deref().unwrap_or("")).await{Ok((200,t))if verify_response(&t,c)=>Probe::Ready{pid:None},_=>Probe::Conflict("Порт занят, но readyMarker/Vite identity не подтверждены. Чужой процесс не останавливается.".into())}
}
pub async fn open(p: &Profile) -> Result<(), String> {
    runtime::open_browser(p.url.as_deref().ok_or("URL не задан.")?).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_blank_marker_never_confirms_identity() {
        let config = ProcessConfig::default();
        assert!(!verify_response(
            r#"<script src="/@vite/client"></script>"#,
            &config
        ));
    }
    #[test]
    fn no_shell() {
        assert!(validate_node("cmd.exe").is_err());
        assert!(validate_node("node --eval anything").is_err());
        assert!(validate_node("node").is_ok());
    }
    #[tokio::test]
    async fn partial_no_probe() {
        let p:Profile=serde_json::from_value(serde_json::json!({"id":"web","name":"Web","subtitle":"Setup","kind":"process","path":"","accent":"#123456","cover":"blueprint","tags":[],"config":{}})).unwrap();
        let c = validate(&p).unwrap();
        assert!(!issues(&c).is_empty());
        assert!(command(&p, &c, Path::new(r"C:\fixtures\bridge.mjs")).is_err());
        assert!(matches!(
            probe(&reqwest::Client::new(), &p, &c).await,
            Probe::Conflict(_)
        ));
    }
}
