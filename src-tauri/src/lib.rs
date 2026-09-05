mod db;
mod importer;
mod models;

use chrono::Local;
use importer::BackendState;
use models::{
    ImportStatus, MetricFilters, ModelStat, Overview, SourceInfo, TaskRow, TimeseriesPoint,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
fn discover_sources(state: State<'_, BackendState>) -> Result<Vec<SourceInfo>, String> {
    db::discover_sources(&state.db_path)
}

#[tauri::command]
fn start_import(
    app: AppHandle,
    state: State<'_, BackendState>,
    force: Option<bool>,
) -> Result<ImportStatus, String> {
    if force.unwrap_or(false) {
        if state.status().running {
            return Err("索引正在运行，请先暂停后再重建".into());
        }
        db::reset_index(&state.db_path)?;
    }
    importer::start_background_import(app, state.inner().clone());
    Ok(state.status())
}

#[tauri::command]
fn pause_import(state: State<'_, BackendState>) -> ImportStatus {
    state.pause()
}

#[tauri::command]
fn get_import_status(state: State<'_, BackendState>) -> ImportStatus {
    state.status()
}

#[tauri::command]
fn query_overview(
    state: State<'_, BackendState>,
    filters: Option<MetricFilters>,
) -> Result<Overview, String> {
    db::query_overview(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn query_timeseries(
    state: State<'_, BackendState>,
    filters: Option<MetricFilters>,
) -> Result<Vec<TimeseriesPoint>, String> {
    db::query_timeseries(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn query_model_stats(
    state: State<'_, BackendState>,
    filters: Option<MetricFilters>,
) -> Result<Vec<ModelStat>, String> {
    db::query_model_stats(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn query_tasks(
    state: State<'_, BackendState>,
    filters: Option<MetricFilters>,
    limit: Option<i64>,
) -> Result<Vec<TaskRow>, String> {
    db::query_tasks(&state.db_path, filters.unwrap_or_default(), limit)
}

#[tauri::command(rename_all = "camelCase")]
fn update_source(
    state: State<'_, BackendState>,
    source_id: i64,
    enabled: bool,
) -> Result<SourceInfo, String> {
    db::update_source(&state.db_path, source_id, enabled)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let refresh = MenuItem::with_id(app, "refresh", "刷新", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Agent Meter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&refresh, &settings, &quit])?;
    let mut builder = TrayIconBuilder::with_id("agent-meter-tray")
        .title("今日 0 · 首响 -- · TPS --")
        .tooltip("Agent Meter")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "refresh" => {
                let state = app.state::<BackendState>();
                importer::start_background_import(app.clone(), state.inner().clone());
            }
            "settings" => {
                show_main_window(app);
                if let Some(window) = app.get_webview_window("main") {
                    use tauri::Emitter;
                    let _ = window.emit("open-settings", ());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn update_tray_title(app: &AppHandle, state: &BackendState) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let filters = MetricFilters {
        start_date: Some(today.clone()),
        end_date: Some(today),
        ..MetricFilters::default()
    };
    let Ok(today_overview) = db::query_overview(&state.db_path, filters) else {
        return;
    };
    let recent_overview = db::query_overview(&state.db_path, MetricFilters::default()).ok();
    let token = compact_number(today_overview.tokens.total);
    let ttft = recent_overview
        .as_ref()
        .and_then(|overview| overview.recent_median_ttft_ms)
        .map(|value| {
            if value >= 1000.0 {
                format!("{:.1}s", value / 1000.0)
            } else {
                format!("{value:.0}ms")
            }
        })
        .unwrap_or_else(|| "--".into());
    let tps = recent_overview
        .as_ref()
        .and_then(|overview| overview.recent_median_effective_tps)
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "--".into());
    if let Some(tray) = app.tray_by_id("agent-meter-tray") {
        let _ = tray.set_title(Some(format!("今日 {token} · 首响 {ttft} · TPS {tps}")));
    }
}

fn compact_number(value: i64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn begin_background_loop(app: AppHandle, state: BackendState) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher: Option<RecommendedWatcher> = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .ok();

        if let Some(active_watcher) = watcher.as_mut() {
            for path in watch_paths(&state) {
                if let Err(error) = active_watcher.watch(&path, RecursiveMode::Recursive) {
                    use tauri::Emitter;
                    let _ = app.emit(
                        "source-error",
                        format!("无法监听 {}: {error}", path.display()),
                    );
                }
            }
        }

        update_tray_title(&app, &state);
        importer::start_background_import(app.clone(), state.clone());

        loop {
            let should_scan = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(Ok(event)) => event.paths.iter().any(|path| db::is_rollout(path)),
                Ok(Err(error)) => {
                    use tauri::Emitter;
                    let _ = app.emit("source-error", format!("文件监听错误: {error}"));
                    true
                }
                Err(RecvTimeoutError::Timeout) => true,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            while rx.try_recv().is_ok() {}
            if should_scan {
                update_tray_title(&app, &state);
                importer::start_background_import(app.clone(), state.clone());
            }
        }
    });
}

fn watch_paths(state: &BackendState) -> Vec<PathBuf> {
    let Ok(sources) = db::enabled_sources(&state.db_path) else {
        return Vec::new();
    };
    sources
        .into_iter()
        .filter(|(_, root)| root.is_dir())
        .flat_map(|(_, root)| {
            let session_dirs = [root.join("sessions"), root.join("archived_sessions")]
                .into_iter()
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            if session_dirs.is_empty() {
                vec![root]
            } else {
                session_dirs
            }
        })
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_data = app.path().app_data_dir()?;
            let db_path = app_data.join("agent-meter.sqlite3");
            db::migrate(&db_path).map_err(std::io::Error::other)?;
            let state = BackendState::new(db_path);
            app.manage(state.clone());
            setup_tray(app)?;
            begin_background_loop(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            discover_sources,
            start_import,
            pause_import,
            get_import_status,
            query_overview,
            query_timeseries,
            query_model_stats,
            query_tasks,
            update_source
        ])
        .build(tauri::generate_context!())
        .expect("error while building Agent Meter");

    app.run(|app, event| {
        if let tauri::RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn compact_numbers_fit_menu_bar() {
        assert_eq!(compact_number(999), "999");
        assert_eq!(compact_number(1_500), "1.5K");
        assert_eq!(compact_number(2_000_000), "2.0M");
    }

    #[test]
    fn watch_paths_prefers_codex_session_directories() {
        let temp = tempdir().unwrap();
        let root = temp.path().join(".codex");
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("archived_sessions")).unwrap();
        let db_path = temp.path().join("meter.sqlite3");
        db::migrate(&db_path).unwrap();
        let conn = db::open(&db_path).unwrap();
        conn.execute("DELETE FROM sources", []).unwrap();
        conn.execute(
            "INSERT INTO sources(name, root_path, enabled) VALUES ('Test', ?1, 1)",
            [root.to_string_lossy().as_ref()],
        )
        .unwrap();
        drop(conn);

        let paths = watch_paths(&BackendState::new(db_path));
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&root.join("sessions")));
        assert!(paths.contains(&root.join("archived_sessions")));
    }
}
