use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

async fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let socket = cng_core::config::rpc_socket_path().map_err(|error| error.to_string())?;
    cng_core::rpc::call(&socket, method, params)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn guard_status() -> Result<Value, String> {
    rpc("status", Value::Null).await
}

#[tauri::command]
async fn refresh_guard() -> Result<Value, String> {
    rpc("refresh", Value::Null).await
}

#[tauri::command]
async fn set_paused(paused: bool) -> Result<Value, String> {
    rpc("pause", serde_json::json!({ "paused": paused })).await
}

#[tauri::command]
async fn remote_action(action: String) -> Result<Value, String> {
    if !matches!(action.as_str(), "start" | "stop" | "pair") {
        return Err("invalid remote action".into());
    }
    rpc(&format!("remote.{action}"), Value::Null).await
}

#[tauri::command]
async fn doctor() -> Result<Value, String> {
    rpc("doctor", Value::Null).await
}

#[tauri::command]
async fn export_diagnostic() -> Result<String, String> {
    let report = doctor().await?;
    let destination = dirs::desktop_dir()
        .or_else(|| cng_core::config::app_support_dir().ok())
        .ok_or_else(|| "cannot determine a location for the diagnostic report".to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let path = destination.join(format!("cng-diagnostic-{stamp}.json"));
    let encoded = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    cng_core::config::write_private(&path, &encoded).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn set_upstream(url: String) -> Result<Value, String> {
    let value = url.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        rpc("upstream.auto", Value::Null).await
    } else {
        rpc("upstream.set", serde_json::json!({ "url": value })).await
    }
}

#[tauri::command]
async fn service_status() -> Result<Value, String> {
    cng_core::service::status()
        .await
        .and_then(|value| serde_json::to_value(value).map_err(Into::into))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_guard() -> Result<Value, String> {
    cng_core::service::install()
        .await
        .and_then(|value| serde_json::to_value(value).map_err(Into::into))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn migrate_legacy_guard() -> Result<Value, String> {
    cng_core::service::migrate_legacy()
        .await
        .and_then(|value| serde_json::to_value(value).map_err(Into::into))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn onboarding_probe() -> Result<Value, String> {
    let config = cng_core::GuardConfig::load_or_create().map_err(|error| error.to_string())?;
    let codex_path = cng_core::codex::find_real_codex(Some(&config));
    let candidates = cng_core::discovery::discover(&config)
        .await
        .map_err(|error| error.to_string())?;
    let health = cng_core::proxy::refresh_health(
        candidates,
        &[],
        std::time::Duration::from_millis(config.connect_timeout_ms),
    )
    .await;
    let healthy = health
        .iter()
        .filter(|value| value.state == cng_core::HealthState::Healthy)
        .count();
    Ok(serde_json::json!({
        "codex_found": codex_path.is_some(),
        "codex_path": codex_path.map(|path| path.display().to_string()),
        "candidate_count": health.len(),
        "healthy_count": healthy,
    }))
}

#[tauri::command]
async fn open_codex() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = tokio::process::Command::new("/usr/bin/open")
        .args(["-b", "com.openai.codex"])
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    let status = tokio::process::Command::new("cmd")
        .args(["/C", "start", "", "codex"])
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    return Err("opening Codex from the desktop app is not implemented on this platform".into());
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Codex could not be opened".to_string())
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            guard_status,
            refresh_guard,
            set_paused,
            remote_action,
            doctor,
            export_diagnostic,
            set_upstream,
            service_status,
            install_guard,
            migrate_legacy_guard,
            onboarding_probe,
            open_codex
        ])
        .setup(|app| {
            let show =
                MenuItem::with_id(app, "show", "Open Codex Network Guard", true, None::<&str>)?;
            let refresh =
                MenuItem::with_id(app, "refresh", "Refresh connection", true, None::<&str>)?;
            let codex = MenuItem::with_id(app, "codex", "Open Codex", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit menu app", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &refresh, &codex, &quit])?;
            TrayIconBuilder::new()
                .tooltip("Codex Network Guard")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "refresh" => {
                        tauri::async_runtime::spawn(async {
                            let _ = rpc("refresh", Value::Null).await;
                        });
                    }
                    "codex" => {
                        tauri::async_runtime::spawn(async {
                            let _ = open_codex().await;
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Codex Network Guard desktop app");
}
