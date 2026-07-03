pub mod clipboard;
pub mod crypto;
pub mod storage;
pub mod db;
pub mod network;
pub mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent, Emitter};

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
async fn get_known_devices(state: tauri::State<'_, AppState>) -> Result<Vec<crate::db::KnownPeer>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_known_peers().map_err(|e| e.to_string())
}

#[tauri::command]
async fn unpair_device(state: tauri::State<'_, AppState>, app_handle: tauri::AppHandle, device_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_peer_sync_state(&device_id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("peer_list_updated", ());
    Ok(())
}
#[tauri::command]
async fn get_history(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let clips = db.get_all_clips().map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    let app_dir = app_handle.path().app_data_dir().unwrap_or_default();
    
    for (id, content_type, encrypted_payload, timestamp, pinned, is_locked, has_attachment, raw_attachment_path) in clips {
        let mut abs_path: Option<String> = None;
        let mut pure_uuid: Option<String> = None;
        if has_attachment {
            if let Some(raw_path) = &raw_attachment_path {
                let extracted = std::path::Path::new(raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !extracted.is_empty() {
                    pure_uuid = Some(extracted.clone());
                    let mut path = app_dir.join("attachments").join(format!("{}.png", extracted));
                    if !path.exists() {
                        let legacy_path = app_dir.join("attachments").join(format!("{}.bin", extracted));
                        if legacy_path.exists() {
                            let _ = std::fs::rename(&legacy_path, &path);
                        } else {
                            path = legacy_path;
                        }
                    }
                    abs_path = Some(path.to_string_lossy().to_string());
                }
            }
        }

        if let Ok(decrypted) = state.crypto.decrypt(&encrypted_payload) {
            if let Ok(content_str) = String::from_utf8(decrypted) {
                result.push(serde_json::json!({
                    "id": id,
                    "content_type": content_type,
                    "content": content_str,
                    "timestamp": timestamp,
                    "pinned": pinned,
                    "is_locked": is_locked,
                    "has_attachment": has_attachment,
                    "attachment_path": abs_path,
                    "attachment_uuid": pure_uuid
                }));
            }
        } else if has_attachment {
            // Attachment-only clip — file download may be pending or it's a locally copied image without inline text.
            // Emit a minimal placeholder row so the UI shows something and can render the image.
            result.push(serde_json::json!({
                "id": id,
                "content_type": content_type,
                "content": "",
                "timestamp": timestamp,
                "pinned": pinned,
                "is_locked": is_locked,
                "has_attachment": true,
                "attachment_path": abs_path,
                "attachment_uuid": pure_uuid
            }));
        }
    }

    Ok(result)
}

#[tauri::command]
async fn toggle_pin(state: State<'_, AppState>, id: i64, pinned: bool) -> Result<(), String> {
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.toggle_pin(id, pinned).map_err(|e| e.to_string())?;
    }
    state.network.trigger_sync(state.db.clone());

    Ok(())
}

#[tauri::command]
async fn toggle_clip_lock(state: State<'_, AppState>, id: i64, is_locked: bool) -> Result<(), String> {
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.toggle_lock(id, is_locked).map_err(|e| e.to_string())?;
    }
    state.network.trigger_sync(state.db.clone());

    Ok(())
}

#[tauri::command]
async fn delete_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_clip(id).map_err(|e| e.to_string())?;
    }
    state.network.trigger_sync(state.db.clone());

    Ok(())
}

#[tauri::command]
async fn get_deleted_clips(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let clips = db.get_deleted_clips().map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    let app_dir = app_handle.path().app_data_dir().unwrap_or_default();
    
    for (id, content_type, encrypted_payload, timestamp, pinned, is_locked, has_attachment, raw_attachment_path) in clips {
        let mut abs_path: Option<String> = None;
        let mut pure_uuid: Option<String> = None;
        if has_attachment {
            if let Some(raw_path) = raw_attachment_path {
                let extracted = std::path::Path::new(&raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !extracted.is_empty() {
                    pure_uuid = Some(extracted.clone());
                    let path = app_dir.join("attachments").join(format!("{}.bin", extracted));
                    abs_path = Some(path.to_string_lossy().to_string());
                }
            }
        }

        if let Ok(decrypted) = state.crypto.decrypt(&encrypted_payload) {
            if let Ok(content_str) = String::from_utf8(decrypted) {
                result.push(serde_json::json!({
                    "id": id,
                    "content_type": content_type,
                    "content": content_str,
                    "timestamp": timestamp,
                    "pinned": pinned,
                    "is_locked": is_locked,
                    "has_attachment": has_attachment,
                    "attachment_path": abs_path,
                    "attachment_uuid": pure_uuid
                }));
            }
        } else if has_attachment {
            // Attachment-only clip — file download may be pending.
            // Emit a minimal placeholder row so the UI shows something.
            result.push(serde_json::json!({
                "id": id,
                "content_type": content_type,
                "content": "",
                "timestamp": timestamp,
                "pinned": pinned,
                "is_locked": is_locked,
                "has_attachment": true,
                "attachment_path": abs_path,
                "attachment_uuid": pure_uuid
            }));
        }
    }

    Ok(result)
}

#[tauri::command]
async fn restore_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.restore_clip(id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn permanently_delete_clip(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.permanently_delete_clip(id).map_err(|e| e.to_string())
}


#[tauri::command]
async fn get_connected_peers(state: State<'_, AppState>) -> Result<Vec<crate::network::PeerInfo>, String> {
    Ok(state.network.get_connected_peers())
}

#[tauri::command]
async fn disconnect_peer(state: State<'_, AppState>, ip: String) -> Result<(), String> {
    state.network.disconnect_peer(&ip);
    Ok(())
}

#[tauri::command]
async fn clear_blocks(state: State<'_, AppState>) -> Result<(), String> {
    state.network.clear_blocks();
    Ok(())
}

#[tauri::command]
async fn empty_recycle_bin(state: State<'_, AppState>) -> Result<(), String> {
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
        
    // Immediately drop any old peers discovered with the previous key
    state.network.clear_peers();
    // Also clear any disconnected/blocked peers so scanning a new QR code reconnects everyone
    state.network.clear_blocks();
    // Clear known peers from DB
    if let Ok(db) = state.db.lock() {
        let _ = db.clear_all_peers();
    }

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
async fn add_mobile_clip(state: State<'_, AppState>, text: String) -> Result<bool, String> {
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

    db_guard.insert_clip("text", &encrypted_payload, limit, false, None).map_err(|e| e.to_string())?;
    drop(db_guard);
    state.network.trigger_sync(state.db.clone());

    Ok(true)
}

#[tauri::command]
async fn add_mobile_image(state: tauri::State<'_, AppState>, app_handle: tauri::AppHandle, bytes: Vec<u8>) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    
    // Encrypt the image bytes
    let encrypted_bytes = state.crypto.encrypt(&bytes).map_err(|e| e.to_string())?;
    
    // Generate UUID
    let uuid = uuid::Uuid::new_v4().to_string();
    
    // Save to attachments/<uuid>.enc
    use tauri::Manager;
    let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    let storage = crate::storage::StorageManager::new(app_data_dir).map_err(|e| e.to_string())?;
    let enc_path = storage.get_encrypted_attachment_path(&uuid);
    std::fs::write(&enc_path, &encrypted_bytes).map_err(|e| e.to_string())?;
    
    // Create base64 thumbnail
    let mut thumbnail = vec![];
    if let Ok(img) = image::load_from_memory(&bytes) {
        let resized = img.resize(100, 100 * 10, image::imageops::FilterType::Triangle);
        let mut cursor = std::io::Cursor::new(&mut thumbnail);
        let _ = resized.write_to(&mut cursor, image::ImageFormat::Jpeg);
    }
    use base64::{Engine as _, engine::general_purpose};
    let b64_thumbnail = general_purpose::STANDARD.encode(&thumbnail);
    let encrypted_thumbnail = state.crypto.encrypt(b64_thumbnail.as_bytes()).unwrap_or_default();
    
    let limit = state.settings.get().history_limit;
    let db_guard = state.db.lock().unwrap();
    db_guard.insert_clip("image", &encrypted_thumbnail, limit, true, Some(uuid)).map_err(|e| e.to_string())?;
    drop(db_guard);
    
    state.network.trigger_sync(state.db.clone());
    
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

#[tauri::command]
async fn get_attachment_path_str(app_handle: tauri::AppHandle, uuid: String) -> Result<String, String> {
    use tauri::Manager;
    let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    let storage = crate::storage::StorageManager::new(app_data_dir).map_err(|e| e.to_string())?;
    
    let path = storage.get_attachment_path(&uuid);
    if path.exists() {
        Ok(path.to_string_lossy().to_string())
    } else {
        let legacy = storage.get_legacy_attachment_path(&uuid);
        if legacy.exists() {
            Ok(legacy.to_string_lossy().to_string())
        } else {
            Err("Attachment not found locally.".into())
        }
    }
}

#[tauri::command]
async fn get_attachment_bytes(app_handle: tauri::AppHandle, state: tauri::State<'_, AppState>, uuid: String, max_width: Option<u32>) -> Result<tauri::ipc::Response, String> {
    use tauri::Manager;
    let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    let storage = crate::storage::StorageManager::new(app_data_dir).map_err(|e| e.to_string())?;
    
    // If a max_width is requested (e.g. mobile), check for a cached resized version first
    if let Some(width) = max_width {
        let mobile_path = storage.attachments_dir.join(format!("{}-w{}.jpg", uuid, width));
        if mobile_path.exists() {
            if let Ok(cached_bytes) = std::fs::read(&mobile_path) {
                return Ok(tauri::ipc::Response::new(cached_bytes));
            }
        }
    }

    let enc_path = storage.get_encrypted_attachment_path(&uuid);
    let path = storage.get_attachment_path(&uuid);
    let legacy = storage.get_legacy_attachment_path(&uuid);
    
    let bytes = if enc_path.exists() {
        let encrypted_bytes = std::fs::read(&enc_path).map_err(|e| format!("Failed to read {}: {}", enc_path.display(), e))?;
        state.crypto.decrypt(&encrypted_bytes).map_err(|e| e.to_string())?
    } else if path.exists() {
        std::fs::read(&path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
    } else {
        std::fs::read(&legacy).map_err(|e| format!("File not found: {} and {}", path.display(), legacy.display()))?
    };
    
    // Generate the cached resized version if requested
    if let Some(width) = max_width {
        let mobile_path = storage.attachments_dir.join(format!("{}-w{}.jpg", uuid, width));
        if let Ok(img) = image::load_from_memory(&bytes) {
            if img.width() > width {
                // Lanczos3 provides much better quality for downscaling
                let resized = img.resize(width, width * 10, image::imageops::FilterType::Lanczos3);
                let mut cursor = std::io::Cursor::new(Vec::new());
                if resized.write_to(&mut cursor, image::ImageFormat::Jpeg).is_ok() {
                    let result_bytes = cursor.into_inner();
                    let _ = std::fs::write(&mobile_path, &result_bytes); // Cache it!
                    return Ok(tauri::ipc::Response::new(result_bytes));
                }
            }
        }
    }

    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn export_attachment(app_handle: tauri::AppHandle, state: tauri::State<'_, AppState>, uuid: String, destination_type: String) -> Result<String, String> {
    use tauri::Manager;
    let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    let storage = crate::storage::StorageManager::new(app_data_dir).map_err(|e| e.to_string())?;
    
    let enc_path = storage.get_encrypted_attachment_path(&uuid);
    let path = storage.get_attachment_path(&uuid);
    let legacy = storage.get_legacy_attachment_path(&uuid);

    let bytes = if enc_path.exists() {
        let encrypted_bytes = std::fs::read(&enc_path).map_err(|e| format!("Failed to read {}: {}", enc_path.display(), e))?;
        state.crypto.decrypt(&encrypted_bytes).map_err(|e| e.to_string())?
    } else if path.exists() {
        std::fs::read(&path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
    } else if legacy.exists() {
        std::fs::read(&legacy).map_err(|e| format!("File not found: {}", legacy.display()))?
    } else {
        return Err("Attachment not found locally.".into());
    };

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let filename = if destination_type == "share" {
        format!("cipherclip-share-{}.png", timestamp)
    } else {
        format!("cipherclip-{}.png", timestamp)
    };
    
    let dest_dir = if destination_type == "share" {
        app_handle.path().document_dir().unwrap_or_else(|_| std::path::PathBuf::from("/storage/emulated/0/Documents"))
    } else {
        app_handle.path().download_dir().unwrap_or_else(|_| std::path::PathBuf::from("/storage/emulated/0/Download"))
    };
    
    let dest_path = dest_dir.join(&filename);
    std::fs::write(&dest_path, &bytes).map_err(|e| format!("Failed to save file: {}", e))?;
    
    Ok(dest_path.to_string_lossy().to_string())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
async fn copy_attachment(app_handle: tauri::AppHandle, state: tauri::State<'_, AppState>, path: String, content_type: String) -> Result<(), String> {
    use clipboard_rs::{Clipboard, ClipboardContext, common::RustImage};

    if content_type == "image" {
        let uuid = std::path::Path::new(&path).file_stem().unwrap_or_default().to_string_lossy().to_string();
        
        use tauri::Manager;
        let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
        let storage = crate::storage::StorageManager::new(app_data_dir).map_err(|e| e.to_string())?;
        
        let enc_path = storage.get_encrypted_attachment_path(&uuid);
        let plain_path = storage.get_attachment_path(&uuid);
        let legacy = storage.get_legacy_attachment_path(&uuid);
        
        let bytes = if enc_path.exists() {
            let encrypted_bytes = std::fs::read(&enc_path).map_err(|e| format!("Failed to read {}: {}", enc_path.display(), e))?;
            state.crypto.decrypt(&encrypted_bytes).map_err(|e| e.to_string())?
        } else if plain_path.exists() {
            std::fs::read(&plain_path).map_err(|e| format!("Failed to read {}: {}", plain_path.display(), e))?
        } else {
            std::fs::read(&legacy).map_err(|e| format!("File not found: {}", path))?
        };
        
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("{}.png", uuid));
        std::fs::write(&temp_file, &bytes).map_err(|e| e.to_string())?;
        
        let res = if let Ok(img) = RustImage::from_path(temp_file.to_string_lossy().as_ref()) {
            let (tx, rx) = std::sync::mpsc::channel();
            app_handle.run_on_main_thread(move || {
                let r = match ClipboardContext::new() {
                    Ok(ctx) => ctx.set_image(img).map_err(|e| format!("Failed to set image clipboard: {}", e)),
                    Err(e) => Err(format!("Failed to init clipboard: {}", e))
                };
                let _ = tx.send(r);
            }).map_err(|e| e.to_string())?;
            rx.recv().unwrap_or(Err("Failed to receive from main thread".to_string()))
        } else {
            Err("Failed to load image from temp path".to_string())
        };
        
        let _ = std::fs::remove_file(temp_file);
        res?;
        Ok(())
    } else {
        let file_uri = if path.starts_with("file://") {
            path
        } else {
            #[cfg(windows)]
            {
                format!("file:///{}", path.replace("\\", "/"))
            }
            #[cfg(not(windows))]
            {
                format!("file://{}", path)
            }
        };
        
        let (tx, rx) = std::sync::mpsc::channel();
        app_handle.run_on_main_thread(move || {
            let r = match ClipboardContext::new() {
                Ok(ctx) => ctx.set_files(vec![file_uri.clone()]).map_err(|e| format!("Clipboard error: {}", e)),
                Err(e) => Err(format!("Failed to init clipboard: {}", e))
            };
            let _ = tx.send(r);
        }).map_err(|e| e.to_string())?;
        rx.recv().unwrap_or(Err("Failed to receive from main thread".to_string()))
    }
}

#[tauri::command]
async fn copy_image_from_base64(app_handle: tauri::AppHandle, base64: String) -> Result<(), String> {
    use clipboard_rs::{Clipboard, ClipboardContext, common::RustImage};
    use base64::{Engine as _, engine::general_purpose};
    
    let bytes = general_purpose::STANDARD.decode(base64).map_err(|e| e.to_string())?;
    
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("{}.png", uuid::Uuid::new_v4().to_string()));
    std::fs::write(&temp_file, &bytes).map_err(|e| e.to_string())?;
    
    let res = if let Ok(img) = RustImage::from_path(temp_file.to_string_lossy().as_ref()) {
        let (tx, rx) = std::sync::mpsc::channel();
        app_handle.run_on_main_thread(move || {
            let r = match ClipboardContext::new() {
                Ok(ctx) => ctx.set_image(img).map_err(|e| format!("Failed to set image clipboard: {}", e)),
                Err(e) => Err(format!("Failed to init clipboard: {}", e))
            };
            let _ = tx.send(r);
        }).map_err(|e| e.to_string())?;
        rx.recv().unwrap_or(Err("Failed to receive from main thread".to_string()))
    } else {
        Err("Failed to load image from temp path".to_string())
    };
    
    let _ = std::fs::remove_file(temp_file);
    res?;
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
async fn copy_attachment(_app_handle: tauri::AppHandle, _path: String, _content_type: String) -> Result<(), String> {
    Err("copy_attachment is not supported on mobile".to_string())
}



#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_share::init())
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

            let settings = Arc::new(SettingsManager::new(app_dir.clone()));
            let db = Arc::new(Mutex::new(Database::new(app_dir.clone(), settings.get().device_id.clone()).unwrap()));
            let crypto = Arc::new(CryptoState::new(&app_dir).unwrap());
            let app_handle_for_cb = app.handle().clone();
        let ui_callback: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync> = Arc::new(move |event, payload| {
            let _ = app_handle_for_cb.emit(event, payload);
        });
        
        let app_data_dir = app.handle().path().app_data_dir().unwrap_or_default();
        
        let network = Arc::new(NetworkManager::new(
            crypto.clone(),
            db.clone(),
            settings.clone(),
            app_data_dir,
            ui_callback,
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

            // ── Mobile: periodic sync poll ──
            // Android may block incoming TCP connections from the desktop,
            // so mobile must actively pull from peers every few seconds.
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                let network_poll = network.clone();
                let db_poll = db.clone();
                let app_handle_poll = app.handle().clone();
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        network_poll.trigger_sync(db_poll.clone());
                        let _ = app_handle_poll.emit("sync-poll", ());
                    }
                });
            }


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
            add_mobile_image,
            get_attachment_bytes,
            get_attachment_path_str,
            toggle_pin,
            delete_clip,
            get_settings,
            set_limit,
            set_shortcut,
            hide_window,
            paste_to_active_window,
            clear_history,
            set_ignore_next_update,
            unpair_device,
        get_known_devices,
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
            copy_attachment,
            copy_image_from_base64,
            export_attachment,
            get_connected_peers,
            disconnect_peer,
            clear_blocks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    include!("tests/sync_protocol_tests.rs");
    include!("tests/data_plane_tests.rs");
    include!("tests/e2e_cluster_tests.rs");
}
