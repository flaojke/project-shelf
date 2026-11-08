use crate::{
    adapters,
    models::{Profile, ProfileValidation, ProjectKind, Settings},
};
use std::path::{Path, PathBuf};
pub const ALLOWED_IDS: [&str; 4] = ["web", "powershell", "docker", "android"];
pub fn absolute_path(v: &str) -> Result<PathBuf, String> {
    let b = v.as_bytes();
    if v.len() > 2048
        || b.len() < 3
        || !b[0].is_ascii_alphabetic()
        || b[1] != b':'
        || !matches!(b[2], b'\\' | b'/')
        || v.chars().any(char::is_control)
        || v.chars()
            .any(|c| matches!(c, '"' | '<' | '>' | '|' | '?' | '*'))
    {
        return Err("Нужен безопасный локальный абсолютный путь.".into());
    }
    if v[2..].contains(':')
        || v[3..]
            .split(['\\', '/'])
            .any(|part| part == ".." || part == "." || part.ends_with(' ') || part.ends_with('.'))
    {
        return Err("Нужен абсолютный путь без переходов '..'.".into());
    }
    Ok(PathBuf::from(v))
}
pub fn checked_child(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Каталог проекта не найден.")?;
    let child = root
        .join(relative)
        .canonicalize()
        .map_err(|_| format!("Не найден необходимый файл: {relative}."))?;
    if !child.starts_with(&root) || !child.is_file() {
        return Err("Файл запуска находится вне каталога проекта.".into());
    }
    Ok(child)
}
pub fn loopback_url(v: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(v).map_err(|_| "Некорректный URL.")?;
    if u.scheme() != "http"
        || !matches!(u.host_str(), Some("127.0.0.1") | Some("localhost"))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(
            "Разрешён только HTTP URL localhost/127.0.0.1 без credentials/query/fragment.".into(),
        );
    }
    Ok(u)
}
pub fn matching_root_url(value: Option<&str>, port: u16) -> Result<(), String> {
    if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
        let u = loopback_url(value)?;
        if u.path() != "/" || u.port_or_known_default() != Some(port) {
            return Err(
                "URL должен указывать на loopback root и совпадать с настроенным портом.".into(),
            );
        }
    }
    Ok(())
}
pub fn profile(p: &Profile) -> Result<(), String> {
    let expected = match p.id.as_str() {
        "web" => ProjectKind::Process,
        "powershell" => ProjectKind::Powershell,
        "docker" => ProjectKind::Docker,
        "android" => ProjectKind::Android,
        _ => return Err("Неизвестный слот проекта.".into()),
    };
    if p.kind != expected {
        return Err("Тип фиксированного слота менять нельзя.".into());
    }
    if !p.path.trim().is_empty() {
        absolute_path(&p.path)?;
    }
    for (name, value, limit) in [
        ("Название", &p.name, 100),
        ("Описание", &p.subtitle, 300),
        ("Обложка", &p.cover, 160),
    ] {
        if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
            return Err(format!("{name}: некорректное значение."));
        }
    }
    if p.cover.contains("://") || p.cover.contains("..") {
        return Err("Обложка должна быть локальным ресурсом.".into());
    }
    if p.accent.len() != 7
        || !p.accent.starts_with('#')
        || !p.accent[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("Цвет должен быть #RRGGBB.".into());
    }
    if p.tags.len() > 12
        || p.tags
            .iter()
            .any(|t| t.len() > 60 || t.chars().any(char::is_control))
    {
        return Err("Некорректные теги.".into());
    }
    if !p.config.is_object()
        || serde_json::to_vec(&p.config)
            .map_err(|_| "Некорректный JSON.")?
            .len()
            > 16384
    {
        return Err("Нужен небольшой config JSON-объект.".into());
    }
    reject_secrets(&p.config)?;
    if let Some(v) = p.url.as_deref().filter(|s| !s.trim().is_empty()) {
        let u = loopback_url(v)?;
        if u.path() != "/" {
            return Err("URL проекта должен указывать на корень сайта.".into());
        }
    }
    match p.kind {
        ProjectKind::Process => {
            adapters::process::validate(p)?;
        }
        ProjectKind::Powershell => {
            adapters::powershell::validate(p)?;
        }
        ProjectKind::Docker => {
            adapters::docker::validate(p)?;
        }
        ProjectKind::Android => {
            adapters::android::validate(p)?;
        }
    }
    Ok(())
}
pub fn profile_validation(p: &Profile) -> ProfileValidation {
    if let Err(e) = profile(p) {
        return ProfileValidation {
            complete: false,
            issues: vec![e],
        };
    }
    let mut issues = Vec::new();
    if matches!(p.kind, ProjectKind::Process | ProjectKind::Powershell) && p.path.trim().is_empty()
    {
        issues.push("Укажите каталог проекта.".into());
    }
    if p.kind != ProjectKind::Android && p.url.as_deref().unwrap_or("").trim().is_empty() {
        issues.push("Укажите loopback URL проекта.".into());
    }
    match p.kind {
        ProjectKind::Process => issues.extend(adapters::process::issues(
            &adapters::process::validate(p).unwrap(),
        )),
        ProjectKind::Powershell => issues.extend(adapters::powershell::issues(
            &adapters::powershell::validate(p).unwrap(),
        )),
        ProjectKind::Docker => issues.extend(adapters::docker::issues(
            &adapters::docker::validate(p).unwrap(),
        )),
        ProjectKind::Android => issues.extend(adapters::android::issues(
            &adapters::android::validate(p).unwrap(),
        )),
    }
    ProfileValidation {
        complete: issues.is_empty(),
        issues,
    }
}
pub fn require_complete(p: &Profile) -> Result<(), String> {
    let v = profile_validation(p);
    if v.complete {
        Ok(())
    } else {
        Err(format!(
            "Завершите настройку проекта: {}",
            v.issues.join(" ")
        ))
    }
}
fn reject_secrets(v: &serde_json::Value) -> Result<(), String> {
    if let Some(map) = v.as_object() {
        for (k, child) in map {
            let k = k.to_ascii_lowercase();
            if [
                "token",
                "secret",
                "password",
                "apikey",
                "api_key",
                "environment",
                "command",
                "shell",
            ]
            .iter()
            .any(|s| k.contains(s))
                || k == "env"
            {
                return Err("Config не принимает секреты, environment или shell-команды.".into());
            }
            reject_secrets(child)?;
        }
    }
    if let Some(a) = v.as_array() {
        for child in a {
            reject_secrets(child)?;
        }
    }
    Ok(())
}
pub fn settings(s: &Settings) -> Result<(), String> {
    if s.theme != "light" {
        return Err("Доступна светлая тема.".into());
    }
    Ok(())
}
pub fn normalized_path(v: &str) -> String {
    v.trim_start_matches("\\\\?\\")
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase()
}
pub fn plain_local_path(p: &Path) -> Result<PathBuf, String> {
    let v = p.to_str().ok_or("Некорректный путь.")?;
    let v = v.strip_prefix(r"\\?\").unwrap_or(v);
    let b = v.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !matches!(b[2], b'\\' | b'/') {
        return Err("Нужен путь локального диска Windows.".into());
    }
    absolute_path(v)
}
/// While owned work is active only favorite metadata can change.
pub fn editing_allowed(old: &Profile, new: &Profile, owned: bool) -> Result<(), String> {
    if old.id != new.id || old.kind != new.kind {
        return Err("ID и kind слота неизменны.".into());
    }
    if owned {
        let mut a = serde_json::to_value(old).unwrap();
        let mut b = serde_json::to_value(new).unwrap();
        a["favorite"] = serde_json::json!(false);
        b["favorite"] = serde_json::json!(false);
        if a != b {
            return Err("Сначала остановите управляемый ресурс; во время его работы доступно только избранное.".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_paths_reject_ads_devices_wildcards_and_windows_aliases() {
        for value in [
            r"C:\fixtures\file:stream",
            r"C:\fixtures\file|pipe",
            r"C:\fixtures\file?.txt",
            r"C:\fixtures\file*.txt",
            r"C:\fixtures\file<name",
            r"C:\fixtures\file>name",
            r"C:\fixtures\..\file",
            r"C:\fixtures\file. ",
            r"C:\fixtures\file.",
            r"C:relative",
            r"\\server\share",
            r"\\?\C:\fixtures",
        ] {
            assert!(absolute_path(value).is_err(), "invalid local path");
        }
        assert!(absolute_path(r"C:\fixtures\project folder").is_ok());
        assert!(absolute_path("C:/fixtures/project").is_ok());
    }
    fn defaults() -> Vec<Profile> {
        serde_json::from_slice(include_bytes!("../../profiles/defaults.json")).unwrap()
    }
    #[test]
    fn default_profiles_are_safe_but_incomplete() {
        for p in defaults() {
            profile(&p).unwrap();
            assert!(!profile_validation(&p).complete);
            assert!(require_complete(&p).is_err());
        }
    }
    #[test]
    fn no_arbitrary_or_unknown_slot_configuration() {
        let mut p = defaults().remove(0);
        p.config = serde_json::json!({"command":"arbitrary"});
        assert!(profile(&p).is_err());
        p.config = serde_json::json!({"unlistedField":"x"});
        assert!(profile(&p).is_err());
        p.id = "unknown".into();
        assert!(profile(&p).is_err());
    }
    #[test]
    fn port_url_and_loopback_must_agree() {
        let mut p = defaults().remove(0);
        p.path = r"C:\fixtures\web".into();
        p.config = serde_json::json!({"port":5190,"readyMarker":"sample-ready"});
        assert!(profile(&p).is_err());
        p.url = Some("http://127.0.0.1:5190/".into());
        assert!(profile_validation(&p).complete);
        for v in [
            "https://localhost:5190/",
            "http://example.invalid/",
            "http://127.0.0.1:5190/?token=x",
        ] {
            p.url = Some(v.into());
            assert!(profile(&p).is_err());
        }
    }
    #[test]
    fn active_runtime_edits_rejected_favorite_allowed() {
        let p = defaults().remove(0);
        let mut n = p.clone();
        n.favorite = !n.favorite;
        editing_allowed(&p, &n, true).unwrap();
        n.path = r"C:\fixtures\web".into();
        assert!(editing_allowed(&p, &n, true).is_err());
        editing_allowed(&p, &n, false).unwrap();
    }
    #[test]
    fn namespace_strip_is_only_local() {
        assert_eq!(
            plain_local_path(Path::new(r"\\?\C:\fixtures\site")).unwrap(),
            PathBuf::from(r"C:\fixtures\site")
        );
        assert!(plain_local_path(Path::new(r"\\?\UNC\server\share")).is_err());
    }
}
