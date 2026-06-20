#[cfg(not(any(target_os = "android", target_os = "ios")))]
use arboard::Clipboard;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use image::{codecs::webp::WebPEncoder, ImageEncoder, RgbaImage};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::crypto::CryptoState;
use crate::db::Database;
use crate::settings::SettingsManager;
use crate::storage::StorageManager;
use std::sync::atomic::{AtomicBool, Ordering};

/// Threshold constants for Data Plane ingestion routing.
/// Text payloads up to this size are stored inline in SQLite.
const _TEXT_INLINE_MAX: usize = 100 * 1024; // 100 KB
/// Text payloads above this size are routed to the attachment Data Plane.
const TEXT_ATTACHMENT_THRESHOLD: usize = 500 * 1024; // 500 KB

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn start_listener(
    app_handle: AppHandle,
    db: Arc<Mutex<Database>>,
    crypto: Arc<CryptoState>,
    settings: Arc<SettingsManager>,
    ignore_next_update: Arc<AtomicBool>,
    network: Arc<crate::network::NetworkManager>,
) {
    // Resolve the app data directory once for StorageManager
    let app_data_dir = {
        use tauri::Manager;
        app_handle.path().app_data_dir().unwrap_or_default()
    };

    std::thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                println!("Failed to init clipboard: {}", e);
                return;
            }
        };

        // Initialize StorageManager for writing attachments to disk
        let storage = match StorageManager::new(app_data_dir.clone()) {
            Ok(s) => s,
            Err(e) => {
                println!("Failed to init StorageManager for clipboard watcher: {}", e);
                return;
            }
        };

        let mut last_hash: Option<Vec<u8>> = {
            let db_guard = db.lock().unwrap();
            if let Some(encrypted) = db_guard.get_latest_hash().unwrap_or(None) {
                if let Ok(decrypted) = crypto.decrypt(&encrypted) {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&decrypted);
                    Some(hasher.finalize().to_vec())
                } else {
                    None
                }
            } else {
                None
            }
        };

        loop {
            std::thread::sleep(Duration::from_millis(400));

            let mut new_payload = None;
            let mut ctype = "";

            // Try to get text
            if let Ok(text) = clipboard.get_text() {
                if !text.is_empty() {
                    new_payload = Some(text.into_bytes());
                    ctype = "text";
                }
            }

            // If text didn't change or empty, try image
            if new_payload.is_none() {
                if let Ok(image) = clipboard.get_image() {
                    // Try compressing to WebP
                    if let Some(mut img) = RgbaImage::from_raw(
                        image.width as u32,
                        image.height as u32,
                        image.bytes.into_owned(),
                    ) {
                        // Max dimension limit to save storage and retain reasonable quality
                        let max_w = 1920u32;
                        let max_h = 1080u32;
                        if img.width() > max_w || img.height() > max_h {
                            let img_dyn = image::DynamicImage::ImageRgba8(img);
                            img = img_dyn
                                .resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
                                .into_rgba8();
                        }

                        let mut webp_bytes = Vec::new();
                        // Quality 80 for reasonable compression
                        let encoder = WebPEncoder::new_lossless(&mut webp_bytes);
                        if let Ok(_) = encoder.write_image(
                            img.as_raw(),
                            img.width() as u32,
                            img.height() as u32,
                            image::ExtendedColorType::Rgba8,
                        ) {
                            new_payload = Some(webp_bytes);
                            ctype = "image";
                        }
                    }
                }
            }

            if let Some(payload) = new_payload {
                // ── Threshold-based routing decision ──
                // Images always go to the Data Plane attachment path.
                // Large text (>500KB) is routed to attachments to prevent SQLite bloat.
                // Small text (≤100KB) stays inline in the database for instant sync.
                // Text between 100KB-500KB is kept inline (reasonable for SQLite).
                let use_attachment = ctype == "image" || (ctype == "text" && payload.len() > TEXT_ATTACHMENT_THRESHOLD);

                if use_attachment {
                    // ── DATA PLANE PATH: Stream binary to disk ──
                    let attachment_uuid = uuid::Uuid::new_v4().to_string();

                    // Write raw bytes to ~/.cipherclip/attachments/<uuid>.bin
                    if let Err(e) = storage.save_chunk(&attachment_uuid, &payload, false) {
                        println!("Failed to save attachment to disk: {}", e);
                        continue;
                    }

                    // For the DB, we store only a lightweight metadata stub.
                    // For images: base64 thumbnail preview; for large text: a truncated preview.
                    let preview_payload = if ctype == "image" {
                        use base64::{engine::general_purpose, Engine as _};
                        general_purpose::STANDARD.encode(&payload).into_bytes()
                    } else {
                        // Store a truncated preview of the large text for the UI
                        let preview_len = std::cmp::min(payload.len(), 512);
                        let preview = String::from_utf8_lossy(&payload[..preview_len]);
                        format!("[Large file: {} bytes]\n{}", payload.len(), preview).into_bytes()
                    };

                    // Hash the full payload for dedup
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&payload);
                    let current_hash = hasher.finalize().to_vec();

                    if last_hash.as_ref() != Some(&current_hash) {
                        last_hash = Some(current_hash);

                        if ignore_next_update.swap(false, Ordering::SeqCst) {
                            println!("Ignoring clipboard update triggered by CipherClip");
                            // Clean up the attachment we just wrote
                            let _ = storage.delete_attachment(&attachment_uuid);
                            continue;
                        }

                        println!("New clipboard {} detected (attachment path, {} bytes)", ctype, payload.len());

                        let encrypted_payload = match crypto.encrypt(&preview_payload) {
                            Ok(p) => p,
                            Err(e) => {
                                println!("Encryption error: {}", e);
                                let _ = storage.delete_attachment(&attachment_uuid);
                                continue;
                            }
                        };

                        let limit = settings.get().history_limit;

                        let db_guard = db.lock().unwrap();
                        match db_guard.insert_clip(ctype, &encrypted_payload, limit, true, Some(attachment_uuid.clone())) {
                            Ok(_) => {
                                let _ = app_handle.emit("clipboard-update", ());
                                network.trigger_sync(db.clone());
                            }
                            Err(e) => {
                                println!("DB insert error: {}", e);
                                let _ = storage.delete_attachment(&attachment_uuid);
                            }
                        }
                    } else {
                        // Duplicate — clean up the attachment we just wrote
                        let _ = storage.delete_attachment(&attachment_uuid);
                    }
                } else {
                    // ── CONTROL PLANE PATH: Inline text in SQLite ──
                    let stored_payload = payload;

                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&stored_payload);
                    let current_hash = hasher.finalize().to_vec();

                    if last_hash.as_ref() != Some(&current_hash) {
                        last_hash = Some(current_hash);

                        if ignore_next_update.swap(false, Ordering::SeqCst) {
                            println!("Ignoring clipboard update triggered by CipherClip");
                            continue;
                        }

                        println!("New clipboard {} detected (inline, {} bytes)", ctype, stored_payload.len());

                        let encrypted_payload = match crypto.encrypt(&stored_payload) {
                            Ok(p) => p,
                            Err(e) => {
                                println!("Encryption error: {}", e);
                                continue;
                            }
                        };

                        let limit = settings.get().history_limit;

                        let db_guard = db.lock().unwrap();
                        match db_guard.insert_clip(ctype, &encrypted_payload, limit, false, None) {
                            Ok(_) => {
                                // Tell frontend to refresh
                                let _ = app_handle.emit("clipboard-update", ());
                                network.trigger_sync(db.clone());
                            }
                            Err(e) => println!("DB insert error: {}", e),
                        }
                    }
                }
            }
        }
    });
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn start_listener(
    _app_handle: AppHandle,
    _db: Arc<Mutex<Database>>,
    _crypto: Arc<CryptoState>,
    _settings: Arc<SettingsManager>,
    _ignore_next_update: Arc<AtomicBool>,
    _network: Arc<crate::network::NetworkManager>,
) {
    // Mobile OSes do not allow continuous background clipboard polling.
    // The clipboard will be checked when the app is focused via frontend logic.
}

