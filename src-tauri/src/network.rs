use crate::crypto::CryptoState;
use crate::db::Database;
use local_ip_address::local_ip;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::Emitter;

const DISCOVERY_PORT: u16 = 45555;
const TCP_PORT: u16 = 45556;
const MAGIC_WORD: &[u8] = b"CIPHERCLIP_DISCOVER";

pub struct NetworkManager {
    peers: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    blocked_ips: Arc<Mutex<HashSet<String>>>,
}

#[derive(serde::Serialize)]
pub struct PeerInfo {
    ip: String,
    name: String,
}

impl NetworkManager {
    pub fn new(
        app_handle: tauri::AppHandle,
        crypto: Arc<CryptoState>,
        db: Arc<Mutex<Database>>,
    ) -> Self {
        let peers: Arc<Mutex<HashMap<String, (Instant, String)>>> = Arc::new(Mutex::new(HashMap::new()));
        let blocked_ips: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        // 1. UDP Discovery Broadcaster
        let crypto_for_bc = crypto.clone();
        thread::spawn(move || loop {
            if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
                let _ = socket.set_broadcast(true);
                let my_ip = local_ip().unwrap_or_else(|_| "127.0.0.1".parse().unwrap());
                let mut device_name = whoami::devicename().unwrap_or_else(|_| "Unknown Device".to_string());
                if device_name.to_lowercase() == "unknown" || device_name.to_lowercase() == "localhost" || device_name.trim().is_empty() {
                    device_name = "Mobile Device".to_string();
                }
                
                // Get hash of the sync key to only discover peers with the exact same key
                if let Ok(raw_key) = crypto_for_bc.get_key() {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&raw_key);
                    let key_hash = hex::encode(hasher.finalize());
                    
                    let msg = format!("CIPHERCLIP_DISCOVER:{}:{}:{}", key_hash, my_ip, device_name);
                    let _ = socket.send_to(msg.as_bytes(), ("255.255.255.255", DISCOVERY_PORT));
                }
            }
            thread::sleep(std::time::Duration::from_secs(5));
        });

        // 2. UDP Discovery Listener
        let peers_clone = peers.clone();
        let blocked_ips_clone = blocked_ips.clone();
        let crypto_for_listen = crypto.clone();
        thread::spawn(move || {
            if let Ok(socket) = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
                let mut buf = [0; 2048];
                loop {
                    if let Ok((amt, _src)) = socket.recv_from(&mut buf) {
                        if amt > 19 && &buf[0..19] == MAGIC_WORD {
                            let msg = String::from_utf8_lossy(&buf[19..amt]);
                            let parts: Vec<&str> = msg.splitn(4, ':').collect();
                            if parts.len() >= 3 { // MAGIC_WORD is removed, so parts are [ "", key_hash, ip, name ] ? Wait.
                                // format is "CIPHERCLIP_DISCOVER:key_hash:ip:name"
                                // so msg is ":key_hash:ip:name" because it stripped MAGIC_WORD
                                if let Ok(raw_key) = crypto_for_listen.get_key() {
                                    use sha2::{Digest, Sha256};
                                    let mut hasher = Sha256::new();
                                    hasher.update(&raw_key);
                                    let expected_hash = hex::encode(hasher.finalize());
                                    
                                    let received_hash = parts[1];
                                    if received_hash == expected_hash {
                                        let ip = parts[2].to_string();
                                        let name = if parts.len() > 3 { parts[3].to_string() } else { "Unknown Device".to_string() };
                                        
                                        let my_ip = local_ip()
                                            .unwrap_or_else(|_| "127.0.0.1".parse().unwrap())
                                            .to_string();
                                        if ip != my_ip {
                                            let mut p = peers_clone.lock().unwrap();
                                            let blocked = blocked_ips_clone.lock().unwrap();
                                            if !blocked.contains(&ip) {
                                                p.insert(ip, (Instant::now(), name));
                                            }
                                            // Prune inactive peers
                                            p.retain(|_, (time, _)| time.elapsed().as_secs() < 15);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // 3. TCP Server for incoming clips
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(("0.0.0.0", TCP_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let crypto_c = crypto.clone();
                        let db_c = db.clone();
                        let app_c = app_handle.clone();

                        thread::spawn(move || {
                            let mut type_len_buf = [0u8; 1];
                            if stream.read_exact(&mut type_len_buf).is_err() {
                                return;
                            }

                            let type_len = type_len_buf[0] as usize;
                            let mut type_buf = vec![0u8; type_len];
                            if stream.read_exact(&mut type_buf).is_err() {
                                return;
                            }
                            let content_type = String::from_utf8_lossy(&type_buf).to_string();

                            let mut len_buf = [0u8; 4];
                            if stream.read_exact(&mut len_buf).is_err() {
                                return;
                            }
                            let payload_len = u32::from_be_bytes(len_buf) as usize;

                            // Protect against massive payloads crashing the app
                            if payload_len > 50 * 1024 * 1024 {
                                return;
                            } // max 50MB

                            let mut payload = vec![0u8; payload_len];
                            if stream.read_exact(&mut payload).is_err() {
                                return;
                            }

                            // Ensure it actually decrypts cleanly with our local key before saving
                            if let Ok(decrypted) = crypto_c.decrypt(&payload) {
                                if content_type == "EVENT" {
                                    if let Ok(event_str) = String::from_utf8(decrypted) {
                                        if let Ok(db_lock) = db_c.lock() {
                                            if event_str.starts_with("PIN:") {
                                                let hash = &event_str[4..];
                                                let _ = db_lock.toggle_pin_by_hash(hash, true);
                                                let _ = app_c.emit("clipboard-update", ());
                                            } else if event_str.starts_with("UNPIN:") {
                                                let hash = &event_str[6..];
                                                let _ = db_lock.toggle_pin_by_hash(hash, false);
                                                let _ = app_c.emit("clipboard-update", ());
                                            } else if event_str.starts_with("DELETE:") {
                                                let hash = &event_str[7..];
                                                let _ = db_lock.delete_clip_by_hash(hash);
                                                let _ = app_c.emit("clipboard-update", ());
                                            }
                                        }
                                    }
                                    return;
                                }

                                if let Ok(db_lock) = db_c.lock() {
                                    if let Ok(Some(latest)) = db_lock.get_latest_hash() {
                                        if latest == payload {
                                            return; // Avoid infinite loops
                                        }
                                    }

                                    // It decrypted cleanly, meaning the sender has the same Sync Key. Save it.
                                    let _ = db_lock.insert_clip(&content_type, &payload, 100);
                                    let _ = app_c.emit("clipboard-update", ());
                                }
                            }
                        });
                    }
                }
            }
        });

        Self { peers, blocked_ips }
    }

    pub fn push_clip(&self, content_type: &str, encrypted_payload: &[u8]) {
        let peers: Vec<String> = self.peers.lock().unwrap().keys().cloned().collect();
        for peer_ip in peers {
            thread::spawn({
                let ct = content_type.to_string();
                let payload = encrypted_payload.to_vec();
                move || {
                    if let Ok(mut stream) = TcpStream::connect((peer_ip.as_str(), TCP_PORT)) {
                        let type_bytes = ct.as_bytes();
                        let mut msg = Vec::new();
                        msg.push(type_bytes.len() as u8);
                        msg.extend_from_slice(type_bytes);

                        let payload_len = payload.len() as u32;
                        msg.extend_from_slice(&payload_len.to_be_bytes());
                        msg.extend_from_slice(&payload);

                        let _ = stream.write_all(&msg);
                    }
                }
            });
        }
    }

    pub fn get_connected_peers(&self) -> Vec<PeerInfo> {
        let mut p = self.peers.lock().unwrap(); p.retain(|_, (time, _)| time.elapsed().as_secs() < 15);
        p.iter()
            .map(|(ip, (_, name))| PeerInfo {
                ip: ip.clone(),
                name: name.clone(),
            })
            .collect()
    }

    pub fn disconnect_peer(&self, ip: &str) {
        if let Ok(mut p) = self.peers.lock() {
            p.remove(ip);
        }
        if let Ok(mut blocked) = self.blocked_ips.lock() {
            blocked.insert(ip.to_string());
        }
    }

    pub fn clear_blocks(&self) {
        if let Ok(mut blocked) = self.blocked_ips.lock() {
            blocked.clear();
        }
    }
}
