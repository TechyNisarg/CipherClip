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
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn start_listener(
    app_handle: AppHandle,
    db: Arc<Mutex<Database>>,
    crypto: Arc<CryptoState>,
    settings: Arc<SettingsManager>,
    ignore_next_update: Arc<AtomicBool>,
    network: Arc<crate::network::NetworkManager>,
) {
    std::thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                println!("Failed to init clipboard: {}", e);
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
                // If image, we need to convert to base64 for frontend
                let stored_payload = if ctype == "image" {
                    use base64::{engine::general_purpose, Engine as _};
                    general_purpose::STANDARD.encode(&payload).into_bytes()
                } else {
                    payload
                };

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

                    println!("New clipboard {} detected!", ctype);

                    let encrypted_payload = match crypto.encrypt(&stored_payload) {
                        Ok(p) => p,
                        Err(e) => {
                            println!("Encryption error: {}", e);
                            continue;
                        }
                    };

                    let limit = settings.get().history_limit;

                    let db_guard = db.lock().unwrap();
                    match db_guard.insert_clip(ctype, &encrypted_payload, limit) {
                        Ok(_) => {
                            // Tell frontend to refresh
                            let _ = app_handle.emit("clipboard-update", ());

                            // Push to network peers!
                            network.push_clip(ctype, &encrypted_payload);
                        }
                        Err(e) => println!("DB insert error: {}", e),
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
