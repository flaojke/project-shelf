mod adapters;
mod diagnostics;
mod models;
mod runtime;
mod store;
mod supervisor;
mod validate;

use models::{LogEntry, Profile, ProjectStatus, Settings};
use std::sync::{atomic::Ordering, Arc};
use supervisor::{QuitSummary, Supervisor};
use tauri::{Emitter, Manager, State};
type Backend = Arc<Supervisor>;

#[tauri::command]
async fn list_profiles(state: State<'_, Backend>) -> Result<Vec<Profile>, String> {
    Ok(state.profiles().await)
}
#[tauri::command]
async fn get_statuses(state: State<'_, Backend>) -> Result<Vec<ProjectStatus>, String> {
    Ok(state.statuses().await)
}
#[tauri::command]
async fn start_project(state: State<'_, Backend>, id: String) -> Result<ProjectStatus, String> {
    state.start(&id).await
}
#[tauri::command]
async fn project_diagnostics(
    state: State<'_, Backend>,
    id: String,
) -> Result<models::ProjectDiagnostics, String> {
    state.diagnostics(&id).await
}
#[tauri::command]
async fn restart_project(state: State<'_, Backend>, id: String) -> Result<ProjectStatus, String> {
    state.restart(&id).await
}
#[tauri::command]
async fn stop_project(state: State<'_, Backend>, id: String) -> Result<ProjectStatus, String> {
    state.stop(&id).await
}
#[tauri::command]
async fn open_project(state: State<'_, Backend>, id: String) -> Result<ProjectStatus, String> {
    state.open(&id).await
}
#[tauri::command]
async fn project_logs(state: State<'_, Backend>, id: String) -> Result<Vec<LogEntry>, String> {
    state.logs(&id).await
}
#[tauri::command]
async fn save_profile(state: State<'_, Backend>, profile: Profile) -> Result<Profile, String> {
    state.save_profile(profile).await
}
#[tauri::command]
fn profile_validation(profile: Profile) -> models::ProfileValidation {
    validate::profile_validation(&profile)
}
#[tauri::command]
async fn remove_profile(_state: State<'_, Backend>, id: String) -> Result<(), String> {
    let _ = id;
    Err("В этой версии доступны четыре фиксированных проекта; удаление отключено.".into())
}
#[tauri::command]
async fn discover_projects(
    _state: State<'_, Backend>,
    root: String,
) -> Result<Vec<Profile>, String> {
    let _ = root;
    Err("Автоматическое добавление проектов отключено для этой версии.".into())
}
#[tauri::command]
async fn get_settings(state: State<'_, Backend>) -> Result<Settings, String> {
    Ok(state.settings().await)
}
#[tauri::command]
async fn save_settings(state: State<'_, Backend>, settings: Settings) -> Result<Settings, String> {
    state.save_settings(settings).await
}
#[tauri::command]
async fn android_devices(state: State<'_, Backend>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.android_devices().await?.into_iter().map(|device|serde_json::json!({"serial":device.serial,"state":device.state,"model":device.label})).collect())
}
#[tauri::command]
async fn install_android_apk(
    state: State<'_, Backend>,
    id: String,
    serial: String,
) -> Result<(), String> {
    state.install_android(&id, &serial).await
}
#[tauri::command]
async fn hide_window(window: tauri::Window) -> Result<(), String> {
    window
        .hide()
        .map_err(|_| "Не удалось скрыть окно в трей.".into())
}
#[tauri::command]
async fn quit_app(
    app: tauri::AppHandle,
    state: State<'_, Backend>,
    stop_owned: bool,
) -> Result<QuitSummary, String> {
    let summary = state.quit_summary(stop_owned).await?;
    // A partial stop is reviewable in the UI. Only an explicit subsequent
    // Leave/Exit request may abandon the remaining resources.
    if !stop_owned || (summary.failed.is_empty() && summary.retained.is_empty()) {
        state.mark_exiting();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            app.exit(0);
        });
    }
    Ok(summary)
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            let backend = Supervisor::new(root).map_err(std::io::Error::other)?;
            app.manage(Arc::new(backend));
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                "Показать Project Shelf",
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Выход…", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let mut tray = tauri::tray::TrayIconBuilder::with_id("project-shelf")
                .tooltip("Project Shelf")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => {
                        show_main(app);
                        let _ = app.emit("request-quit", ());
                    }
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _tray = tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Some(state) = window.app_handle().try_state::<Backend>() {
                    if !state.exiting.load(Ordering::SeqCst) {
                        api.prevent_close();
                        let _ = window.emit("request-close", ());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            get_statuses,
            start_project,
            project_diagnostics,
            restart_project,
            stop_project,
            open_project,
            project_logs,
            save_profile,
            profile_validation,
            remove_profile,
            discover_projects,
            get_settings,
            save_settings,
            android_devices,
            install_android_apk,
            quit_app,
            hide_window
        ])
        .run(tauri::generate_context!())
        .expect("Project Shelf could not initialize its native runtime");
}
