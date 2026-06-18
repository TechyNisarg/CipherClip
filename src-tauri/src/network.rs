use crate::crypto::CryptoState;
use crate::db::Database;
use local_ip_address::local_ip;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::Emitter;

const DISCOVERY_PORT: u16 = 45555;
const TCP_PORT: u16 = 45556;

pub struct NetworkManager {
    peers: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    blocked_ips: Arc<Mutex<HashSet<String>>>,
    crypto: Arc<CryptoState>,
    instance_id: String,
    settings: Arc<crate::settings::SettingsManager>,
}

#[derive(serde::Serialize)]
pub struct PeerInfo {
    ip: String,
    name: String,
}

/// Get the local IP, trying multiple strategies for reliability on Android/hotspot
fn get_my_ip() -> String {
    // Try the local_ip crate first
    if let Ok(ip) = local_ip() {
        let ip_str = ip.to_string();
        if ip_str != "127.0.0.1" {
            return ip_str;
        }
    }
    // Fallback: connect to a public IP (doesn't actually send data) to discover local interface
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        // Connect to Google's DNS — this doesn't send packets, just picks the right interface
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

use std::sync::atomic::{AtomicBool, Ordering};
static NETWORK_INITIALIZED: AtomicBool = AtomicBool::new(false);

impl NetworkManager {
    pub fn new(
        app_handle: tauri::AppHandle,
        crypto: Arc<CryptoState>,
        db: Arc<Mutex<Database>>,
        settings: Arc<crate::settings::SettingsManager>,
    ) -> Self {
        let instance_id_str = rand::random::<u32>().to_string();
        let peers: Arc<Mutex<HashMap<String, (Instant, String)>>> = Arc::new(Mutex::new(HashMap::new()));
        
        let blocked_ips_set = settings.get_blocked_ips().into_iter().collect();
        let blocked_ips: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(blocked_ips_set));

        if NETWORK_INITIALIZED.swap(true, Ordering::SeqCst) {
            println!("NetworkManager already initialized! Skipping thread spawn.");
            return Self { peers, blocked_ips, crypto: crypto.clone(), instance_id: instance_id_str.clone(), settings: settings.clone() };
        }

        // 1. UDP Discovery Broadcaster
        let crypto_for_bc = crypto.clone();
        let instance_id_bc = instance_id_str.clone();
        thread::spawn(move || {
            // Create socket once outside the loop
            let socket = match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to bind broadcaster socket: {}", e);
                    return;
                }
            };
            let _ = socket.set_broadcast(true);

            loop {
                let my_ip = get_my_ip();
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
                    
                    let msg = format!("CIPHERCLIP_DISCOVER:{}:{}:{}:{}", key_hash, instance_id_bc, my_ip, device_name);
                    let msg_bytes = msg.as_bytes();

                    // Send to global broadcast (works on most networks)
                    let _ = socket.send_to(msg_bytes, ("255.255.255.255", DISCOVERY_PORT));

                    // Also try common hotspot subnet broadcasts for reliability
                    // Mobile hotspot typically uses 192.168.x.255
                    if let Ok(ip) = my_ip.parse::<std::net::Ipv4Addr>() {
                        let octets = ip.octets();
                        // Subnet broadcast for /24 (most common for hotspot and home WiFi)
                        let subnet_broadcast = format!("{}.{}.{}.255", octets[0], octets[1], octets[2]);
                        if subnet_broadcast != "255.255.255.255" {
                            let _ = socket.send_to(msg_bytes, (subnet_broadcast.as_str(), DISCOVERY_PORT));
                        }
                    }
                }
                thread::sleep(std::time::Duration::from_secs(3)); // Broadcast every 3s for faster discovery
            }
        });

        // 2. UDP Discovery Listener
        let peers_clone = peers.clone();
        let blocked_ips_clone = blocked_ips.clone();
        let crypto_for_listen = crypto.clone();
        let instance_id_listen = instance_id_str.clone();
        let settings_clone = settings.clone();
        thread::spawn(move || {
            let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
                Ok(s) => {
                    // Allow multiple binds to the same port (important for Android)
                    let _ = s.set_broadcast(true);
                    s
                }
                Err(e) => {
                    log::error!("Failed to bind discovery listener on port {}: {}", DISCOVERY_PORT, e);
                    return;
                }
            };

            let mut buf = [0; 2048];
            let prefix = b"CIPHERCLIP_DISCOVER:";
            let prefix_len = prefix.len(); // 20 bytes including the colon

            loop {
                if let Ok((amt, src_addr)) = socket.recv_from(&mut buf) {
                    // Check for our protocol prefix (CIPHERCLIP_DISCOVER:)
                    if amt > prefix_len && &buf[0..prefix_len] == prefix {
                        let msg = String::from_utf8_lossy(&buf[prefix_len..amt]);
                        let parts: Vec<&str> = msg.splitn(4, ':').collect();
                        if parts.len() >= 3 {
                            if let Ok(raw_key) = crypto_for_listen.get_key() {
                                use sha2::{Digest, Sha256};
                                let mut hasher = Sha256::new();
                                hasher.update(&raw_key);
                                let expected_hash = hex::encode(hasher.finalize());
                                
                                let received_hash = parts[0]; // key_hash
                                if received_hash == expected_hash {
                                    let received_instance_id = parts[1]; // instance_id
                                    if received_instance_id != instance_id_listen {
                                        let ip = parts[2].to_string(); // ip from the message
                                        let name = if parts.len() > 3 { parts[3].to_string() } else { "Unknown Device".to_string() };
                                        
                                        // Use the source address from the packet as the actual peer IP
                                        // This is more reliable than the IP reported in the message,
                                        // especially on NAT/hotspot setups
                                        let actual_peer_ip = match src_addr {
                                            SocketAddr::V4(addr) => addr.ip().to_string(),
                                            SocketAddr::V6(addr) => addr.ip().to_string(),
                                        };

                                        let mut p = peers_clone.lock().unwrap();
                                        let blocked = blocked_ips_clone.lock().unwrap();
                                        if !blocked.contains(&actual_peer_ip) {
                                            // Use actual_peer_ip (from packet source) for TCP connections
                                            p.insert(actual_peer_ip, (Instant::now(), name));
                                        }
                                        // Prune inactive peers (not seen in 15 seconds)
                                        p.retain(|_, (time, _)| time.elapsed().as_secs() < 15);
                                    }
                                }
                            }
                        }
                    } else if amt > 22 && &buf[0..22] == b"CIPHERCLIP_DISCONNECT:" {
                        let msg = String::from_utf8_lossy(&buf[22..amt]);
                        let parts: Vec<&str> = msg.splitn(3, ':').collect();
                        if parts.len() == 3 {
                            if let Ok(raw_key) = crypto_for_listen.get_key() {
                                use sha2::{Digest, Sha256};
                                let mut hasher = Sha256::new();
                                hasher.update(&raw_key);
                                let expected_hash = hex::encode(hasher.finalize());
                                
                                let received_hash = parts[0];
                                if received_hash == expected_hash {
                                    let mut p = peers_clone.lock().unwrap();
                                    let mut blocked = blocked_ips_clone.lock().unwrap();
                                    
                                    let actual_peer_ip = match src_addr {
                                        SocketAddr::V4(addr) => addr.ip().to_string(),
                                        SocketAddr::V6(addr) => addr.ip().to_string(),
                                    };
                                    
                                    p.remove(&actual_peer_ip);
                                    if blocked.insert(actual_peer_ip.clone()) {
                                        settings_clone.add_blocked_ip(actual_peer_ip);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // 3. TCP Server for incoming clips
        let crypto_tcp = crypto.clone();
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(("0.0.0.0", TCP_PORT)) {
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        let crypto_c = crypto_tcp.clone();
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
                                            } else if event_str.starts_with("LOCK:") {
                                                let hash = &event_str[5..];
                                                let _ = db_lock.toggle_lock_by_hash(hash, true);
                                                let _ = app_c.emit("clipboard-update", ());
                                            } else if event_str.starts_with("UNLOCK:") {
                                                let hash = &event_str[7..];
                                                let _ = db_lock.toggle_lock_by_hash(hash, false);
                                                let _ = app_c.emit("clipboard-update", ());
                                            } else if event_str.starts_with("DELETE:") {
                                                let hash = &event_str[7..];
                                                // Immunity from rule #2: Ignore if locked
                                                if let Ok(is_locked) = db_lock.is_locked_by_hash(hash) {
                                                    if !is_locked {
                                                        let _ = db_lock.delete_clip_by_hash(hash);
                                                        let _ = app_c.emit("clipboard-update", ());
                                                    }
                                                }
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

        Self { peers, blocked_ips, crypto, instance_id: instance_id_str, settings }
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
        let mut p = self.peers.lock().unwrap();
        p.retain(|_, (time, _)| time.elapsed().as_secs() < 15);
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
            if blocked.insert(ip.to_string()) {
                self.settings.add_blocked_ip(ip.to_string());
            }
        }
        
        // Broadcast a DISCONNECT to this peer explicitly
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            if let Ok(raw_key) = self.crypto.get_key() {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&raw_key);
                let key_hash = hex::encode(hasher.finalize());
                let my_ip = get_my_ip();
                let msg = format!("CIPHERCLIP_DISCONNECT:{}:{}:{}", key_hash, self.instance_id, my_ip);
                let _ = socket.send_to(msg.as_bytes(), format!("{}:{}", ip, DISCOVERY_PORT));
            }
        }
    }

    pub fn clear_blocks(&self) {
        if let Ok(mut blocked) = self.blocked_ips.lock() {
            blocked.clear();
            self.settings.clear_blocked_ips();
        }
    }

    pub fn clear_peers(&self) {
        if let Ok(mut p) = self.peers.lock() {
            p.clear();
        }
    }
}
