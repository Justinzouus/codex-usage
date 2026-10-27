mod analytics;
mod billing;
mod codex;
mod models;
mod runtime_paths;

use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use models::{ProviderSnapshot, WidgetPreferences};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, Rect,
    State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const CACHE_TTL: Duration = Duration::from_secs(30);
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const POPOVER_BLUR_DELAY: Duration = Duration::from_millis(120);
const POPOVER_TRAY_SETTLE: Duration = Duration::from_millis(350);
const POPOVER_SIZE: f64 = 288.0;
const POPOVER_GAP: f64 = 2.0;
const TRAY_PLACEHOLDER: &str = "--%  ↻--";

struct AppState {
    preferences: Mutex<WidgetPreferences>,
    preferences_path: PathBuf,
    billing_settings_path: PathBuf,
    billing_file_lock: Mutex<()>,
    fetch_lock: tokio::sync::Mutex<()>,
    snapshot_cache: Mutex<Option<(Instant, Vec<ProviderSnapshot>)>>,
    tray_title: Mutex<String>,
    last_tray_toggle: Mutex<Instant>,
}

fn cached_snapshots(state: &AppState, max_age: Option<Duration>) -> Option<Vec<ProviderSnapshot>> {
    state.snapshot_cache.lock().ok().and_then(|cache| {
        cache.as_ref().and_then(|(time, values)| {
            if max_age.is_none_or(|age| time.elapsed() < age) {
                Some(values.clone())
            } else {
                None
            }
        })
    })
}

async fn load_snapshots(state: &AppState, force: bool) -> Vec<ProviderSnapshot> {
    if !force {
        if let Some(values) = cached_snapshots(state, Some(CACHE_TTL)) {
            return values;
        }
    }

    let _guard = match state.fetch_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let _waiting_guard = state.fetch_lock.lock().await;
            return cached_snapshots(state, None).unwrap_or_else(|| {
                vec![ProviderSnapshot::failure(
                    "unavailable",
                    "Quota refresh did not produce a snapshot.",
                )]
            });
        }
    };

    if !force {
        if let Some(values) = cached_snapshots(state, Some(CACHE_TTL)) {
            return values;
        }
    }

    let values = vec![codex::fetch_snapshot().await];
    if let Ok(mut cache) = state.snapshot_cache.lock() {
        *cache = Some((Instant::now(), values.clone()));
    }
    values
}

fn load_preferences(path: &PathBuf) -> WidgetPreferences {
    let parse = |candidate: &PathBuf| {
        fs::read_to_string(candidate)
            .ok()
            .and_then(|raw| serde_json::from_str::<WidgetPreferences>(&raw).ok())
    };
    if let Some(value) = parse(path) {
        return value.normalized();
    }
    let backup = path.with_extension("json.bak");
    if let Some(value) = parse(&backup) {
        eprintln!("preferences recovered from backup");
        return value.normalized();
    }
    WidgetPreferences::default()
}

fn persist_preferences(path: &PathBuf, value: &WidgetPreferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "failed to create settings directory".to_string())?;
    }
    let serialized =
        serde_json::to_vec_pretty(value).map_err(|_| "failed to serialize settings".to_string())?;
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    let mut file = fs::File::create(&temporary)
        .map_err(|_| "failed to create temporary settings file".to_string())?;
    file.write_all(&serialized)
        .and_then(|_| file.sync_all())
        .map_err(|_| "failed to write settings".to_string())?;
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|_| "failed to back up settings".to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        return Err(format!("failed to commit settings: {error}"));
    }
    Ok(())
}

#[tauri::command]
async fn get_snapshots(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderSnapshot>, String> {
    let values = load_snapshots(state.inner(), false).await;
    publish_snapshots(&app, state.inner(), &values);
    Ok(values)
}

#[tauri::command]
async fn refresh_snapshots(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderSnapshot>, String> {
    let values = load_snapshots(state.inner(), true).await;
    publish_snapshots(&app, state.inner(), &values);
    Ok(values)
}

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<WidgetPreferences, String> {
    state
        .preferences
        .lock()
        .map(|value| value.clone())
        .map_err(|_| "settings unavailable".into())
}

#[tauri::command]
fn set_preferences(
    preferences: WidgetPreferences,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let preferences = preferences.normalized();
    persist_preferences(&state.preferences_path, &preferences)?;
    *state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())? = preferences;
    Ok(())
}

#[tauri::command]
fn get_billing_settings(
    state: State<'_, AppState>,
) -> Result<Option<billing::BillingSettings>, String> {
    let _guard = state
        .billing_file_lock
        .lock()
        .map_err(|_| "账期设置暂不可用")?;
    billing::load(&state.billing_settings_path)
}

#[tauri::command]
fn set_billing_settings(
    settings: billing::BillingSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _guard = state
        .billing_file_lock
        .lock()
        .map_err(|_| "账期设置暂不可用")?;
    billing::save(&state.billing_settings_path, &settings)
}

#[tauri::command]
fn hide_popover(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("widget")
        .ok_or_else(|| "details window missing".to_string())?
        .hide()
        .map_err(|error| format!("failed to hide details: {error}"))
}

#[tauri::command]
async fn get_analytics(
    app: AppHandle,
    query: analytics::types::Query,
) -> Result<analytics::types::Dashboard, String> {
    let force = query.refresh_pricing.unwrap_or(false);
    let service = app.state::<analytics::pricing::PricingService>();
    let snapshot = service.refresh(force).await;
    let dashboard = query_analytics(app.clone(), query.clone(), snapshot).await?;
    if dashboard.pricing.missing_models.is_empty() {
        return Ok(dashboard);
    }
    let snapshot = service
        .resolve_missing(&dashboard.pricing.missing_models, force)
        .await;
    query_analytics(app, query, snapshot).await
}
async fn query_analytics(
    app: AppHandle,
    query: analytics::types::Query,
    snapshot: analytics::pricing::PricingSnapshot,
) -> Result<analytics::types::Dashboard, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<analytics::AnalyticsState>();
        let current = state
            .current_window
            .lock()
            .map_err(|_| "额度周期暂不可用")?
            .clone();
        let mut store = state.store.lock().map_err(|_| "历史统计暂不可用")?;
        store.pricing = snapshot;
        store.query(
            &analytics::codex_home()?,
            query,
            chrono::Utc::now().timestamp(),
            current,
        )
    })
    .await
    .map_err(|_| "历史统计加载失败".to_string())?
}

#[tauri::command]
fn open_pricing_source(url: String) -> Result<(), String> {
    if !analytics::pricing::is_pricing_url(&url) {
        return Err("只允许打开已验证的 OpenAI 官方价格来源".into());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("无法打开官方来源：{e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("当前平台暂不支持打开官方价格来源".into())
    }
}

#[tauri::command]
fn open_dashboard(app: AppHandle) -> Result<(), String> {
    if let Some(popover) = app.get_webview_window("widget") {
        let _ = popover.hide();
    }
    if let Some(window) = app.get_webview_window("dashboard") {
        window.show().map_err(|_| "无法显示用量大盘")?;
        window.set_focus().map_err(|_| "无法聚焦用量大盘")?;
    } else {
        tauri::WebviewWindowBuilder::new(
            &app,
            "dashboard",
            tauri::WebviewUrl::App("index.html?dashboard".into()),
        )
        .title("Codex Usage · 使用概览")
        .inner_size(1100.0, 820.0)
        .min_inner_size(720.0, 560.0)
        .resizable(true)
        .decorations(true)
        .transparent(false)
        .center()
        .build()
        .map_err(|_| "无法创建用量大盘窗口")?;
    }
    Ok(())
}

fn conversation_window_route(thread_id: &str) -> Result<(String, String), String> {
    if thread_id.is_empty()
        || thread_id.len() > 128
        || !thread_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("会话标识无效".into());
    }
    Ok((
        format!("conversation-{thread_id}"),
        format!("index.html?conversation={thread_id}"),
    ))
}

#[tauri::command]
fn open_conversation(app: AppHandle, thread_id: String) -> Result<(), String> {
    let (label, route) = conversation_window_route(&thread_id)?;
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|_| "无法显示会话详情")?;
        window.set_focus().map_err(|_| "无法聚焦会话详情")?;
    } else {
        let parent = app
            .get_webview_window("dashboard")
            .ok_or("用量大盘尚未打开")?;
        tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App(route.into()))
            .title("Codex Usage · 会话用量详情")
            .inner_size(960.0, 700.0)
            .min_inner_size(720.0, 500.0)
            .resizable(true)
            .decorations(true)
            .transparent(false)
            .parent(&parent)
            .map_err(|_| "无法关联会话详情窗口")?
            .center()
            .build()
            .map_err(|_| "无法创建会话详情窗口")?;
    }
    Ok(())
}

fn format_tray_title(snapshot: &ProviderSnapshot) -> Option<String> {
    if snapshot.status == "signed_out" {
        return Some(TRAY_PLACEHOLDER.to_string());
    }

    if snapshot.status != "ok" && snapshot.status != "stale" {
        return None;
    }

    let weekly = snapshot.weekly_window.as_ref().map(|window| {
        if window.remaining_percent.is_finite() {
            window.remaining_percent.clamp(0.0, 100.0).round() as u64
        } else {
            0
        }
    });
    let weekly_text = weekly
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "--%".to_string());
    let credits_text = snapshot
        .reset_credits
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_string());
    Some(format!("{weekly_text}  ↻{credits_text}"))
}

fn publish_snapshots(app: &AppHandle, state: &AppState, values: &[ProviderSnapshot]) {
    if let Some(snapshot) = values.iter().find(|item| item.provider == "codex") {
        if snapshot.status == "ok" {
            if let (Some(analytics), Some(window)) = (
                app.try_state::<analytics::AnalyticsState>(),
                snapshot.weekly_window.as_ref(),
            ) {
                if let Some(end) = window
                    .resets_at
                    .as_ref()
                    .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                {
                    if let Ok(mut current) = analytics.current_window.lock() {
                        *current = Some(analytics::types::ResetWindow {
                            end: end.timestamp(),
                            seconds: window.window_seconds as i64,
                        });
                    }
                }
            }
        }
        if let Some(next_title) = format_tray_title(snapshot) {
            if let Ok(mut current_title) = state.tray_title.lock() {
                if *current_title != next_title {
                    if let Some(tray) = app.tray_by_id("main") {
                        let _ = tray.set_title(Some(&next_title));
                    }
                    *current_title = next_title;
                }
            }
        }
    }
    let _ = app.emit_to("widget", "snapshots-updated", values.to_vec());
}

async fn refresh_application_snapshots(app: AppHandle) {
    let state = app.state::<AppState>();
    let values = load_snapshots(state.inner(), true).await;
    publish_snapshots(&app, state.inner(), &values);
}

fn spawn_snapshot_refresh(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh_application_snapshots(app.clone()).await;
            tokio::time::sleep(AUTO_REFRESH_INTERVAL).await;
        }
    });
}

fn fitted_popover_position(
    tray_position: PhysicalPosition<i32>,
    tray_size: PhysicalSize<u32>,
    popover_size: PhysicalSize<u32>,
    work_area: &PhysicalRect<i32, u32>,
    gap: i32,
) -> PhysicalPosition<i32> {
    let min_x = i64::from(work_area.position.x);
    let min_y = i64::from(work_area.position.y);
    let max_x =
        (min_x + i64::from(work_area.size.width) - i64::from(popover_size.width)).max(min_x);
    let max_y =
        (min_y + i64::from(work_area.size.height) - i64::from(popover_size.height)).max(min_y);
    let desired_x = i64::from(tray_position.x)
        + (i64::from(tray_size.width) - i64::from(popover_size.width)) / 2;
    let desired_y = i64::from(tray_position.y) + i64::from(tray_size.height) + i64::from(gap);

    PhysicalPosition::new(
        desired_x.clamp(min_x, max_x) as i32,
        desired_y.clamp(min_y, max_y) as i32,
    )
}

fn position_popover(window: &tauri::WebviewWindow, tray_rect: Rect) -> tauri::Result<()> {
    let fallback_scale = window.scale_factor().unwrap_or(1.0);
    let tray_position = tray_rect.position.to_physical::<i32>(fallback_scale);
    let tray_size = tray_rect.size.to_physical::<u32>(fallback_scale);
    let center_x = f64::from(tray_position.x) + f64::from(tray_size.width) / 2.0;
    let center_y = f64::from(tray_position.y) + f64::from(tray_size.height) / 2.0;
    let monitor = window
        .monitor_from_point(center_x, center_y)?
        .or(window.current_monitor()?)
        .or(window.primary_monitor()?);

    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let popover_size = LogicalSize::new(POPOVER_SIZE, POPOVER_SIZE).to_physical::<u32>(scale);
        let gap = (POPOVER_GAP * scale).round() as i32;
        let position = fitted_popover_position(
            tray_position,
            tray_size,
            popover_size,
            monitor.work_area(),
            gap,
        );
        window.set_size(popover_size)?;
        window.set_position(position)?;
    } else {
        window.set_size(LogicalSize::new(POPOVER_SIZE, POPOVER_SIZE))?;
    }

    Ok(())
}

fn show_popover(app: &AppHandle, tray_rect: Rect) {
    if let Some(window) = app.get_webview_window("widget") {
        if let Err(error) = position_popover(&window, tray_rect) {
            eprintln!("details positioning failed: {error}");
        }
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to("widget", "refresh-requested", ());
    }
}

fn toggle_popover(app: &AppHandle, tray_rect: Rect) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut last_toggle) = state.last_tray_toggle.lock() {
            *last_toggle = Instant::now();
        }
    }

    if let Some(window) = app.get_webview_window("widget") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            show_popover(app, tray_rect);
        }
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let dashboard = MenuItem::with_id(
        app,
        "dashboard",
        "使用大盘 / Usage dashboard",
        true,
        None::<&str>,
    )?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let language = MenuItem::with_id(
        app,
        "language",
        "Switch Language / 切换语言",
        true,
        None::<&str>,
    )?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start at login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &dashboard, &refresh, &language, &autostart, &separator, &quit,
        ],
    )?;
    let autostart_menu = autostart.clone();
    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .title(TRAY_PLACEHOLDER)
        .tooltip("Codex Usage")
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "dashboard" => {
                if open_dashboard(app.clone()).is_err() {
                    eprintln!("dashboard could not be opened");
                }
            }
            "refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(refresh_application_snapshots(app));
            }
            "language" => {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut prefs) = state.preferences.lock() {
                        prefs.language = if prefs.language == "en" {
                            "zh-CN".into()
                        } else {
                            "en".into()
                        };
                        let normalized = prefs.clone().normalized();
                        *prefs = normalized.clone();
                        let _ = persist_preferences(&state.preferences_path, &normalized);
                        let _ = app.emit_to("widget", "preferences-changed", normalized);
                    }
                }
            }
            "autostart" => {
                let manager = app.autolaunch();
                let enabled = manager.is_enabled().unwrap_or(false);
                let result = if enabled {
                    manager.disable()
                } else {
                    manager.enable()
                };
                match result {
                    Ok(()) => {
                        let _ = autostart_menu.set_checked(!enabled);
                    }
                    Err(_) => eprintln!("autostart update failed"),
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                // Focus the popover after AppKit finishes the menu bar click.
                toggle_popover(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(tray) = app.tray_by_id("main") {
                if let Ok(Some(rect)) = tray.rect() {
                    show_popover(app, rect);
                }
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let isolated_directories =
                runtime_paths::isolated_directories().map_err(std::io::Error::other)?;
            let data_dir = match isolated_directories {
                Some(directories) => directories.state.clone(),
                None => app.path().app_config_dir()?,
            };
            let pricing_dir = match isolated_directories {
                Some(directories) => directories.state.clone(),
                None => app.path().app_data_dir()?,
            };
            let preferences_path = data_dir.join("preferences.json");
            let preferences = load_preferences(&preferences_path);
            let analytics_state = analytics::AnalyticsState::default();
            analytics_state
                .store
                .lock()
                .map_err(|_| std::io::Error::other("history unavailable"))?
                .configure_mode_cache(data_dir.join("service-mode-evidence.json"));
            app.manage(analytics_state);
            app.manage(analytics::pricing::PricingService::new(
                pricing_dir.join("api-pricing-cache.json"),
            ));
            app.manage(AppState {
                preferences: Mutex::new(preferences.clone()),
                billing_settings_path: data_dir.join("billing-settings.json"),
                billing_file_lock: Mutex::new(()),
                preferences_path,
                fetch_lock: tokio::sync::Mutex::new(()),
                snapshot_cache: Mutex::new(None),
                tray_title: Mutex::new(TRAY_PLACEHOLDER.to_string()),
                last_tray_toggle: Mutex::new(
                    Instant::now()
                        .checked_sub(Duration::from_secs(1))
                        .unwrap_or_else(Instant::now),
                ),
            });
            if setup_tray(app).is_err() {
                eprintln!("menu bar setup failed");
            }
            if let Some(window) = app.get_webview_window("widget") {
                let _ = window.set_always_on_top(true);
                let _ = window.set_ignore_cursor_events(false);
                let _ = window.hide();
            }
            spawn_snapshot_refresh(app.handle().clone());
            let mode_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let app = mode_app.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || {
                        if let Ok(home) = analytics::codex_home() {
                            if let Ok(mut store) =
                                app.state::<analytics::AnalyticsState>().store.lock()
                            {
                                store.refresh_modes(&home);
                            }
                        }
                    })
                    .await;
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            });
            let pricing_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    pricing_app
                        .state::<analytics::pricing::PricingService>()
                        .refresh(false)
                        .await;
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
            });
            if isolated_directories.is_some() {
                open_dashboard(app.handle().clone()).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_billing_settings,
            set_billing_settings,
            get_snapshots,
            refresh_snapshots,
            get_preferences,
            set_preferences,
            hide_popover,
            open_dashboard,
            open_conversation,
            get_analytics,
            open_pricing_source
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. }
                if !window.label().starts_with("conversation-") =>
            {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(false) if window.label() == "widget" => {
                let window = window.clone();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(POPOVER_BLUR_DELAY).await;
                    let tray_settle_remaining =
                        app.try_state::<AppState>()
                            .and_then(|state| {
                                state.last_tray_toggle.lock().ok().and_then(|time| {
                                    POPOVER_TRAY_SETTLE.checked_sub(time.elapsed())
                                })
                            })
                            .unwrap_or_default();
                    if !tray_settle_remaining.is_zero() {
                        tokio::time::sleep(tray_settle_remaining).await;
                    }
                    if !window.is_focused().unwrap_or(false) {
                        let _ = window.hide();
                    }
                });
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("failed to build Codex Usage");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Resumed) {
            let app = app_handle.clone();
            tauri::async_runtime::spawn(refresh_application_snapshots(app));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UsageWindow;

    #[test]
    fn conversation_window_routes_are_scoped_and_reject_url_injection() {
        assert_eq!(
            conversation_window_route("abc-123").unwrap(),
            (
                "conversation-abc-123".into(),
                "index.html?conversation=abc-123".into()
            )
        );
        for invalid in ["", "../widget", "id?dashboard", "a&b", "a/b"] {
            assert!(conversation_window_route(invalid).is_err());
        }
    }

    fn snapshot(
        status: &str,
        weekly_remaining: Option<f64>,
        reset_credits: Option<u64>,
    ) -> ProviderSnapshot {
        ProviderSnapshot {
            provider: "codex".into(),
            display_name: "CODEX".into(),
            plan: Some("PRO".into()),
            short_window: None,
            weekly_window: weekly_remaining.map(|remaining_percent| UsageWindow {
                remaining_percent,
                resets_at: None,
                window_seconds: 604_800,
            }),
            reset_credits,
            reset_credit_expires_at: Vec::new(),
            updated_at: "2026-07-20T00:00:00Z".into(),
            status: status.into(),
            message: None,
        }
    }

    #[test]
    fn formats_weekly_quota_and_reset_credits_for_the_menu_bar() {
        assert_eq!(
            format_tray_title(&snapshot("ok", Some(97.0), Some(4))).as_deref(),
            Some("97%  ↻4")
        );
        assert_eq!(
            format_tray_title(&snapshot("ok", Some(0.0), Some(0))).as_deref(),
            Some("0%  ↻0")
        );
    }

    #[test]
    fn rounds_and_clamps_menu_bar_percentage() {
        assert_eq!(
            format_tray_title(&snapshot("ok", Some(97.6), Some(2))).as_deref(),
            Some("98%  ↻2")
        );
        assert_eq!(
            format_tray_title(&snapshot("ok", Some(140.0), Some(2))).as_deref(),
            Some("100%  ↻2")
        );
    }

    #[test]
    fn handles_unknown_and_failed_menu_bar_values_without_using_the_5_hour_window() {
        assert_eq!(
            format_tray_title(&snapshot("ok", None, Some(4))).as_deref(),
            Some("--%  ↻4")
        );
        assert!(format_tray_title(&snapshot("unavailable", None, None)).is_none());
        assert_eq!(
            format_tray_title(&snapshot("signed_out", Some(97.0), Some(4))).as_deref(),
            Some(TRAY_PLACEHOLDER)
        );
    }

    #[test]
    fn centers_the_popover_below_the_menu_bar_item() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(0, 48),
            size: PhysicalSize::new(3_840, 2_112),
        };
        assert_eq!(
            fitted_popover_position(
                PhysicalPosition::new(1_800, 0),
                PhysicalSize::new(160, 48),
                PhysicalSize::new(576, 576),
                &work_area,
                4,
            ),
            PhysicalPosition::new(1_592, 52)
        );
    }

    #[test]
    fn keeps_the_popover_inside_the_right_edge() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(0, 48),
            size: PhysicalSize::new(3_840, 2_112),
        };
        assert_eq!(
            fitted_popover_position(
                PhysicalPosition::new(3_600, 0),
                PhysicalSize::new(160, 48),
                PhysicalSize::new(576, 576),
                &work_area,
                4,
            ),
            PhysicalPosition::new(3_264, 52)
        );
    }

    #[test]
    fn supports_negative_coordinates_on_a_left_hand_display() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(-2_560, 48),
            size: PhysicalSize::new(2_560, 1_392),
        };
        assert_eq!(
            fitted_popover_position(
                PhysicalPosition::new(-100, 0),
                PhysicalSize::new(80, 48),
                PhysicalSize::new(288, 288),
                &work_area,
                2,
            ),
            PhysicalPosition::new(-288, 50)
        );
    }
}
