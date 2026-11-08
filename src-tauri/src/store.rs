use crate::{
    models::{Profile, Settings},
    validate,
};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|_| "Не удалось создать AppData Project Shelf.")?;
        Ok(Self { root })
    }
    pub fn load_profiles(&self) -> Result<Vec<Profile>, String> {
        let path = self.root.join("profiles.json");
        let data = if path.exists() {
            fs::read(&path).map_err(|_| "Не удалось прочитать профили.")?
        } else {
            include_bytes!("../../profiles/defaults.json").to_vec()
        };
        if data.len() > 256 * 1024 {
            return Err("Файл профилей слишком велик.".into());
        }
        let profiles: Vec<Profile> = serde_json::from_slice(&data)
            .map_err(|_| "JSON профилей повреждён; файл сохранён без изменений.")?;
        if profiles.len() != 4 {
            return Err("Ожидаются четыре стартовых профиля.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for profile in &profiles {
            validate::profile(profile)?;
            if !ids.insert(&profile.id) {
                return Err("Дублирующийся ID профиля.".into());
            }
        }
        Ok(profiles)
    }
    pub fn load_settings(&self) -> Result<Settings, String> {
        let path = self.root.join("settings.json");
        let settings = if path.exists() {
            let data = fs::read(path).map_err(|_| "Не удалось прочитать настройки.")?;
            if data.len() > 8192 {
                return Err("Файл настроек слишком велик.".into());
            }
            serde_json::from_slice(&data)
                .map_err(|_| "JSON настроек повреждён; файл сохранён без изменений.")?
        } else {
            Settings::default()
        };
        validate::settings(&settings)?;
        Ok(settings)
    }
    pub fn save_profiles(&self, profiles: &[Profile]) -> Result<(), String> {
        self.write("profiles.json", &profiles)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        self.write("settings.json", settings)
    }
    pub fn vite_bridge(&self) -> Result<PathBuf, String> {
        let runtime = Self::new(self.root.join("runtime"))?;
        let bytes = include_bytes!("../resources/vite-supervisor.mjs");
        let path = runtime.root.join("vite-supervisor-v1.mjs");
        if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            runtime.write_raw("vite-supervisor-v1.mjs", bytes)?;
        }
        Ok(path)
    }
    fn write<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<(), String> {
        let bytes =
            serde_json::to_vec_pretty(value).map_err(|_| "Не удалось сериализовать JSON.")?;
        self.write_raw(name, &bytes)
    }
    fn write_raw(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Ошибка системных часов.")?
            .as_nanos();
        let temp = self
            .root
            .join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
        let target = self.root.join(name);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|_| "Не удалось создать временный JSON.")?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Не удалось записать JSON.")?;
            drop(file);
            atomic_replace(&temp, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err("Не удалось атомарно заменить JSON; предыдущая версия сохранена.".into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|_| "Не удалось атомарно заменить JSON.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_bridge_is_repaired_only_inside_own_appdata_runtime() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-data")
            .join(format!(
                "bridge-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
        let store = Store::new(root.clone()).unwrap();
        let path = store.vite_bridge().unwrap();
        assert_eq!(path.parent().unwrap(), root.join("runtime"));
        fs::write(&path, b"incomplete bridge").unwrap();
        assert_eq!(store.vite_bridge().unwrap(), path);
        assert_eq!(
            fs::read(&path).unwrap(),
            include_bytes!("../resources/vite-supervisor.mjs")
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(root.join("runtime")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn replace_preserves_valid_json() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-data")
            .join(format!(
                "shelf-store-test-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
        let store = Store::new(root.clone()).unwrap();
        let first = Settings::default();
        store.save_settings(&first).unwrap();
        let mut second = first;
        second.auto_open_browser = false;
        store.save_settings(&second).unwrap();
        assert!(!store.load_settings().unwrap().auto_open_browser);
        fs::remove_file(root.join("settings.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
