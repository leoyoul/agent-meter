mod analytics;
mod db;
mod importer;
mod metric_icon;
mod models;
mod settings;

use importer::BackendState;
use models::{
    AnalyticsFilters, ImportStatus, MetricFilters, MetricSeriesPoint, MetricSummary,
    ModelEffortStat, ModelStat, Overview, PricingCatalogStatus, SourceInfo, TaskRow,
    TimeseriesPoint,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use settings::{AppSettings, SettingsState};
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

#[tauri::command]
fn query_metric_summary(
    state: State<'_, BackendState>,
    filters: Option<AnalyticsFilters>,
) -> Result<MetricSummary, String> {
    analytics::query_metric_summary(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn query_metric_series(
    state: State<'_, BackendState>,
    filters: Option<AnalyticsFilters>,
) -> Result<Vec<MetricSeriesPoint>, String> {
    analytics::query_metric_series(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn query_model_effort_stats(
    state: State<'_, BackendState>,
    filters: Option<AnalyticsFilters>,
) -> Result<Vec<ModelEffortStat>, String> {
    analytics::query_model_effort_stats(&state.db_path, filters.unwrap_or_default())
}

#[tauri::command]
fn get_pricing_catalog_status(
    state: State<'_, BackendState>,
) -> Result<PricingCatalogStatus, String> {
    analytics::pricing_catalog_status(&state.db_path)
}

#[tauri::command]
fn reprice_usage(state: State<'_, BackendState>) -> Result<PricingCatalogStatus, String> {
    analytics::reprice_usage(&state.db_path)
}

#[tauri::command(rename_all = "camelCase")]
fn update_source(
    state: State<'_, BackendState>,
    source_id: i64,
    enabled: bool,
) -> Result<SourceInfo, String> {
    db::update_source(&state.db_path, source_id, enabled)
}

#[tauri::command]
fn get_app_settings(state: State<'_, SettingsState>) -> AppSettings {
    state.load()
}

#[tauri::command]
fn update_app_settings(
    app: AppHandle,
    backend: State<'_, BackendState>,
    state: State<'_, SettingsState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    state.save(&settings)?;
    sync_metric_trays(&app, &settings)?;
    update_tray_titles(&app, backend.inner());
    Ok(settings)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let refresh = MenuItem::with_id(app, "refresh", "刷新", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Agent Meter", true, None::<&str>)?;
    Menu::with_items(app, &[&refresh, &settings, &quit])
}

fn build_metric_tray(app: &AppHandle, id: &str, title: &str, tooltip: &str) -> tauri::Result<()> {
    let menu = tray_menu(app)?;
    TrayIconBuilder::with_id(id)
        .icon(metric_icon::render(title, "--"))
        .icon_as_template(true)
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .build(app)?;
    Ok(())
}

fn sync_metric_tray(
    app: &AppHandle,
    enabled: bool,
    id: &str,
    title: &str,
    tooltip: &str,
) -> Result<(), String> {
    match (enabled, app.tray_by_id(id)) {
        (true, None) => {
            build_metric_tray(app, id, title, tooltip).map_err(|error| error.to_string())
        }
        (false, Some(_)) => {
            app.remove_tray_by_id(id);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn sync_metric_trays(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    for (id, enabled, title, tooltip) in metric_tray_specs(settings) {
        sync_metric_tray(app, enabled, id, title, tooltip)?;
    }
    Ok(())
}

fn metric_tray_specs(
    settings: &AppSettings,
) -> [(&'static str, bool, &'static str, &'static str); 4] {
    [
        (
            "metric-tokens",
            settings.menu_metrics.today_tokens,
            "量",
            "Token 总量",
        ),
        ("metric-ttft", settings.menu_metrics.ttft, "首", "平均首响"),
        (
            "metric-tps",
            settings.menu_metrics.effective_tps,
            "速",
            "平均有效 TPS",
        ),
        (
            "metric-cost",
            settings.menu_metrics.estimated_cost,
            "费",
            "API 等价费用（USD）",
        ),
    ]
}

fn setup_trays(app: &tauri::App, settings: &AppSettings) -> tauri::Result<()> {
    let menu = tray_menu(app.handle())?;
    let mut builder = TrayIconBuilder::with_id("agent-meter-tray")
        .tooltip("Agent Meter")
        .menu(&menu)
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone()).icon_as_template(true);
    }
    builder.build(app)?;
    sync_metric_trays(app.handle(), settings).map_err(std::io::Error::other)?;

    app.on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            show_main_window(tray.app_handle());
        }
    });
    app.on_menu_event(|app, event| match event.id().as_ref() {
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
    Ok(())
}

fn update_tray_titles(app: &AppHandle, state: &BackendState) {
    let settings = app.state::<SettingsState>().load();
    let Ok(summary) = analytics::query_metric_summary(
        &state.db_path,
        AnalyticsFilters {
            period: settings.menu_period,
            ..AnalyticsFilters::default()
        },
    ) else {
        return;
    };
    let token = compact_number(summary.tokens.total);
    let ttft = summary
        .average_ttft_ms
        .map(|value| {
            if value >= 1000.0 {
                format!("{:.1}s", value / 1000.0)
            } else {
                format!("{value:.0}ms")
            }
        })
        .unwrap_or_else(|| "--".into());
    let tps = summary
        .average_effective_tps
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "--".into());
    let cost = compact_cost(summary.estimated_cost_nano_usd, summary.pricing.complete);
    let period = period_name(settings.menu_period);
    if let Some(tray) = app.tray_by_id("metric-tokens") {
        let _ = tray.set_icon_with_as_template(Some(metric_icon::render("量", &token)), true);
        let _ = tray.set_tooltip(Some(format!("{period} Token 总量 {token}")));
    }
    if let Some(tray) = app.tray_by_id("metric-ttft") {
        let _ = tray.set_icon_with_as_template(Some(metric_icon::render("首", &ttft)), true);
        let _ = tray.set_tooltip(Some(format!("{period}平均首响 {ttft}")));
    }
    if let Some(tray) = app.tray_by_id("metric-tps") {
        let _ = tray.set_icon_with_as_template(Some(metric_icon::render("速", &tps)), true);
        let _ = tray.set_tooltip(Some(format!("{period}平均有效 TPS {tps}")));
    }
    if let Some(tray) = app.tray_by_id("metric-cost") {
        let _ = tray.set_icon_with_as_template(Some(metric_icon::render("费", &cost)), true);
        let _ = tray.set_tooltip(Some(format!(
            "{period} API 等价费用 {cost} · 计价覆盖 {:.0}%",
            summary.pricing.ratio * 100.0
        )));
    }
}

fn period_name(period: models::MetricPeriod) -> &'static str {
    match period {
        models::MetricPeriod::Realtime => "最近 10 次",
        models::MetricPeriod::Today => "今日",
        models::MetricPeriod::Week => "本周",
        models::MetricPeriod::Month => "本月",
        models::MetricPeriod::Year => "本年",
    }
}

fn compact_cost(nano_usd: i64, complete: bool) -> String {
    let usd = nano_usd as f64 / 1_000_000_000.0;
    let value = if usd >= 1_000.0 {
        format!("${:.1}K", usd / 1_000.0)
    } else if usd >= 10.0 {
        format!("${usd:.0}")
    } else if usd >= 1.0 {
        format!("${usd:.1}")
    } else {
        format!("${usd:.2}")
    };
    if complete {
        value
    } else {
        format!("{value}+")
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

        update_tray_titles(&app, &state);
        importer::start_background_import(app.clone(), state.clone());

        loop {
            let should_scan = match rx.recv_timeout(Duration::from_secs(5)) {
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
                update_tray_titles(&app, &state);
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
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_data = app.path().app_data_dir()?;
            let db_path = app_data.join("agent-meter.sqlite3");
            db::migrate(&db_path).map_err(std::io::Error::other)?;
            let state = BackendState::new(db_path);
            app.manage(state.clone());
            let settings_state = SettingsState::new(app_data.join("settings.json"));
            let settings = settings_state.load();
            app.manage(settings_state);
            setup_trays(app, &settings)?;
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
            query_metric_summary,
            query_metric_series,
            query_model_effort_stats,
            get_pricing_catalog_status,
            reprice_usage,
            update_source,
            get_app_settings,
            update_app_settings
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
    fn every_metric_visibility_combination_is_independent() {
        for bits in 0_u8..16 {
            let mut settings = AppSettings::default();
            settings.menu_metrics.today_tokens = bits & 1 != 0;
            settings.menu_metrics.ttft = bits & 2 != 0;
            settings.menu_metrics.effective_tps = bits & 4 != 0;
            settings.menu_metrics.estimated_cost = bits & 8 != 0;
            let enabled = metric_tray_specs(&settings)
                .into_iter()
                .filter(|(_, visible, _, _)| *visible)
                .map(|(id, _, _, _)| id)
                .collect::<Vec<_>>();
            assert_eq!(enabled.len(), bits.count_ones() as usize);
            assert_eq!(enabled.contains(&"metric-tokens"), bits & 1 != 0);
            assert_eq!(enabled.contains(&"metric-ttft"), bits & 2 != 0);
            assert_eq!(enabled.contains(&"metric-tps"), bits & 4 != 0);
            assert_eq!(enabled.contains(&"metric-cost"), bits & 8 != 0);
        }
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
