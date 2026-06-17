pub mod clipboard;
pub mod crypto;
pub mod db;
pub mod network;
pub mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

use crypto::CryptoState;
use db::Database;
use network::NetworkManager;
use settings::{AppSettings, SettingsManager};

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub settings: Arc<SettingsManager>,
    pub crypto: Arc<CryptoState>,
    pub ignore_next_update: Arc<AtomicBool>,
    pub network: Arc<NetworkManager>,
}

#[tauri::command]
fn get_history(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let clips = db.get_all_clips().map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for (id, content_type, encrypted_payload, timestamp, pinned, is_locked) in clips {
        if let Ok(decrypted) = state.crypto.decrypt(&encrypted_payload) {
            if let Ok(content_str) = String::from_utf8(decrypted) {
                result.push(serde_json::json!({
                    "id": id,
                    "content_type": content_type,
                    "content": content_str,
                    "timestamp": timestamp,
                    "pinned": pinned,
                    "is_locked": is_locked
                }));
            }
        }
    }

    Ok(result)
}

#[tauri::command]
fn toggle_pin(state: State<'_, AppState>, id: i64, pinned: bool) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.toggle_pin(id, pinned).map_err(|e| e.to_string())?;

    if let Ok(Some(hash)) = db.get_hash_by_id(id) {
        let event_str = if pinned { format!("PIN:{}", hash) } else { format!("UNPIN:{}", hash) };
        if let Ok(encrypted_event) = state.crypto.encrypt(event_str.as_bytes()) {
            state.network.push_clip("EVENT", &encrypted_event);
        }
    }

    Ok(())
}

#[tauri::command]
fn toggle_clip_lock(state: State<'_, AppState>, id: i64, is_locked: bool) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.toggle_lock(id, is_locked).map_err(|e| e.to_string())?;

    if let Ok(Some(hash)) = db.get_hash_by_id(id) {
        let event_str = if is_locked {
            format!("LOCK:{}", hash)
        } else {
            format!("UNLOCK:{}", hash)
        };
        if let Ok(encrypted_event) = state.crypto.encrypt(event_str.as_bytes()) {
            state.network.push_clip("EVENT", &encrypted_event);
        }
    }

    Ok(())
}

#[tauri::command]
fn delete_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_clip(id).map_err(|e| e.to_string())?;

    if let Ok(Some(hash)) = db.get_hash_by_id(id) {
        let event_str = format!("DELETE:{}", hash);
        if let Ok(encrypted_event) = state.crypto.encrypt(event_str.as_bytes()) {
            state.network.push_clip("EVENT", &encrypted_event);
        }
    }

    Ok(())
}

#[tauri::command]
fn get_deleted_clips(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let clips = db.get_deleted_clips().map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for (id, content_type, encrypted_payload, timestamp, pinned, is_locked) in clips {
        if let Ok(decrypted) = state.crypto.decrypt(&encrypted_payload) {
            if let Ok(content_str) = String::from_utf8(decrypted) {
                result.push(serde_json::json!({
                    "id": id,
                    "content_type": content_type,
                    "content": content_str,
                    "timestamp": timestamp,
                    "pinned": pinned,
                    "is_locked": is_locked
                }));
            }
        }
    }

    Ok(result)
}

#[tauri::command]
fn restore_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.restore_clip(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn permanently_delete_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.permanently_delete_clip(id).map_err(|e| e.to_string())
}


#[tauri::command]
fn get_connected_peers(state: State<'_, AppState>) -> Result<Vec<crate::network::PeerInfo>, String> {
    Ok(state.network.get_connected_peers())
}

#[tauri::command]
fn disconnect_peer(state: State<'_, AppState>, ip: String) -> Result<(), String> {
    state.network.disconnect_peer(&ip);
    Ok(())
}

#[tauri::command]
fn empty_recycle_bin(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.empty_recycle_bin().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(state.settings.get())
}

#[tauri::command]
fn get_sync_key(app_handle: tauri::AppHandle) -> Result<String, String> {
    let app_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    CryptoState::get_key_hex(&app_dir)
}

#[tauri::command]
fn set_sync_key(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    hex_key: String,
) -> Result<(), String> {
    let app_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    state.crypto.set_key_hex(&app_dir, &hex_key)
}

#[tauri::command]
fn set_limit(state: State<'_, AppState>, limit: i64) -> Result<(), String> {
    state.settings.set_limit(limit)
}

#[tauri::command]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn set_shortcut(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    shortcut: String,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let old_shortcut_str = state.settings.get().global_shortcut;

    if let Ok(old_shortcut) = old_shortcut_str.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        let _ = app.global_shortcut().unregister(old_shortcut);
    }

    if let Ok(new_shortcut) = shortcut.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        if let Err(e) = app.global_shortcut().register(new_shortcut) {
            return Err(format!("Failed to register shortcut: {}", e));
        }
    } else {
        return Err("Invalid shortcut format".to_string());
    }

    state.settings.set_shortcut(shortcut)
}

#[tauri::command]
#[cfg(any(target_os = "android", target_os = "ios"))]
fn set_shortcut(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    shortcut: String,
) -> Result<(), String> {
    state.settings.set_shortcut(shortcut)
}

#[tauri::command]
fn set_theme(state: State<'_, AppState>, theme: String) -> Result<(), String> {
    state.settings.set_theme(theme)
}

#[tauri::command]
fn set_master_password(state: State<'_, AppState>, password: Option<String>) -> Result<(), String> {
    if let Some(pwd) = password {
        // Validation: 8 chars, 1 uppercase, 1 lowercase, 1 number, 1 special
        if pwd.len() < 8 {
            return Err("Password must be at least 8 characters long.".to_string());
        }
        if !pwd.chars().any(|c| c.is_uppercase()) {
            return Err("Password must contain at least one uppercase letter.".to_string());
        }
        if !pwd.chars().any(|c| c.is_lowercase()) {
            return Err("Password must contain at least one lowercase letter.".to_string());
        }
        if !pwd.chars().any(|c| c.is_numeric()) {
            return Err("Password must contain at least one number.".to_string());
        }
        if !pwd.chars().any(|c| !c.is_alphanumeric()) {
            return Err("Password must contain at least one special character.".to_string());
        }

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(pwd.as_bytes());
        let hash = hex::encode(hasher.finalize());
        state.settings.set_master_password_hash(Some(hash))
    } else {
        if let Err(e) = state.db.lock().unwrap().clear_all_locks() {
            return Err(format!("Failed to clear locks: {}", e));
        }
        state.settings.set_master_password_hash(None)
    }
}

#[tauri::command]
fn verify_master_password(state: State<'_, AppState>, password: String) -> Result<bool, String> {
    if let Some(stored_hash) = state.settings.get().master_password_hash {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let hash = hex::encode(hasher.finalize());
        Ok(hash == stored_hash)
    } else {
        Ok(true)
    }
}

#[tauri::command]
fn has_master_password(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.settings.get().master_password_hash.is_some())
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[tauri::command]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn paste_to_active_window(state: State<'_, AppState>) {
    state.ignore_next_update.store(true, Ordering::SeqCst);

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(150)); // wait for window to hide
        use enigo::{Enigo, Key, KeyboardControllable};
        let mut enigo = Enigo::new();
        // MacOS uses Cmd, Windows/Linux uses Ctrl
        #[cfg(target_os = "macos")]
        {
            enigo.key_down(Key::Meta);
            enigo.key_click(Key::Layout('v'));
            enigo.key_up(Key::Meta);
        }
        #[cfg(not(target_os = "macos"))]
        {
            enigo.key_down(Key::Control);
            enigo.key_click(Key::Layout('v'));
            enigo.key_up(Key::Control);
        }
    });
}

#[tauri::command]
fn add_mobile_clip(state: State<'_, AppState>, text: String) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    
    let payload = text.into_bytes();
    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let current_hash = hasher.finalize().to_vec();

    let db_guard = state.db.lock().unwrap();
    if let Some(encrypted) = db_guard.get_latest_hash().unwrap_or(None) {
        if let Ok(decrypted) = state.crypto.decrypt(&encrypted) {
            let mut h2 = Sha256::new();
            h2.update(&decrypted);
            if h2.finalize().to_vec() == current_hash {
                return Ok(false); // Already exists
            }
        }
    }

    let encrypted_payload = state.crypto.encrypt(&payload).map_err(|e| e.to_string())?;
    let limit = state.settings.get().history_limit;

    db_guard.insert_clip("text", &encrypted_payload, limit).map_err(|e| e.to_string())?;
    state.network.push_clip("text", &encrypted_payload);

    Ok(true)
}

#[tauri::command]
#[cfg(any(target_os = "android", target_os = "ios"))]
fn paste_to_active_window(state: State<'_, AppState>) {
    state.ignore_next_update.store(true, Ordering::SeqCst);
    // Auto-paste is not supported on mobile OS natively via keyboard simulation
}

#[tauri::command]
fn clear_history(state: State<'_, AppState>, delete_locked: bool) -> Result<(), String> {
    let db_guard = state.db.lock().unwrap();
    db_guard.clear_all(delete_locked).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_ignore_next_update(state: State<'_, AppState>) {
    state.ignore_next_update.store(true, Ordering::SeqCst);
}
#[tauri::command]
fn open_image_preview(base64_data: String) -> Result<(), String> {
    use base64::Engine;
    use std::io::Write;
    let data = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| e.to_string())?;

    let img =
        image::load_from_memory(&data).map_err(|e| format!("Failed to decode image: {}", e))?;
    let path = std::env::temp_dir().join("cipherclip_preview.png");
    img.save(&path)
        .map_err(|e| format!("Failed to save image: {}", e))?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", path.to_str().unwrap()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_log::Builder::new().build());

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder
            .plugin(tauri_plugin_window_state::Builder::default().build())
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_autostart::Builder::new().build())
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, _shortcut, event| {
                        if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                            if let Some(window) = app.get_webview_window("main") {
                                if let (Ok(visible), Ok(focused)) =
                                    (window.is_visible(), window.is_focused())
                                {
                                    if visible && focused {
                                        let _ = window.hide();
                                    } else {
                                        let _ = window.show();
                                        let _ = window.set_focus();
                                    }
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                    })
                    .build(),
            );
    }

    builder
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            let db = Arc::new(Mutex::new(Database::new(app_dir.clone()).unwrap()));
            let crypto = Arc::new(CryptoState::new(&app_dir).unwrap());
            let settings = Arc::new(SettingsManager::new(app_dir.clone()));
            let network = Arc::new(NetworkManager::new(
                app.handle().clone(),
                crypto.clone(),
                db.clone(),
            ));

            let ignore_next_update = Arc::new(AtomicBool::new(false));

            app.manage(AppState {
                db: db.clone(),
                settings: settings.clone(),
                crypto: crypto.clone(),
                ignore_next_update: ignore_next_update.clone(),
                network: network.clone(),
            });

            // Start clipboard listener
            clipboard::start_listener(
                app.handle().clone(),
                db.clone(),
                crypto.clone(),
                settings.clone(),
                ignore_next_update.clone(),
                network.clone(),
            );

            // Register global shortcut
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                if let Ok(shortcut) = settings
                    .get()
                    .global_shortcut
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                {
                    let _ = app.global_shortcut().register(shortcut);
                }

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            if let Some(window) = tray.app_handle().get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_history,
            add_mobile_clip,
            toggle_pin,
            delete_clip,
            get_settings,
            set_limit,
            set_shortcut,
            hide_window,
            paste_to_active_window,
            clear_history,
            set_ignore_next_update,
            get_deleted_clips,
            restore_clip,
            permanently_delete_clip,
            empty_recycle_bin,
            get_sync_key,
            set_sync_key,
            set_theme,
            set_master_password,
            verify_master_password,
            has_master_password,
            toggle_clip_lock,
            open_image_preview,
            get_connected_peers,
            disconnect_peer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
