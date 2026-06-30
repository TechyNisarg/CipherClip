use std::path::PathBuf;
use crate::crypto::CryptoState;
use crate::db::Database;
use local_ip_address::local_ip;
use std::collections::{HashMap, HashSet};
use std::time::{Instant, Duration};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
// use tauri::Emitter;

const DISCOVERY_PORT: u16 = 45555;
const TCP_PORT: u16 = 45556;
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TCP_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Create a TcpStream with connect + read/write timeouts to prevent blocking.
fn connect_tcp(addr: (&str, u16)) -> std::io::Result<TcpStream> {
    let sock_addr = (addr.0, addr.1).to_socket_addrs()?.next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no address"))?;
    let stream = TcpStream::connect_timeout(&sock_addr, TCP_CONNECT_TIMEOUT)?;
    let _ = stream.set_read_timeout(Some(TCP_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(TCP_IO_TIMEOUT));
    Ok(stream)
}

pub struct NetworkManager {
    peers: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    blocked_ips: Arc<Mutex<HashSet<String>>>,
    crypto: Arc<CryptoState>,
    instance_id: String,
    settings: Arc<crate::settings::SettingsManager>,
    last_catchup: Arc<Mutex<HashMap<String, Instant>>>,
    ui_callback: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>,
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
        crypto: Arc<CryptoState>,
        db: Arc<Mutex<Database>>,
        settings: Arc<crate::settings::SettingsManager>,
        app_data_dir: PathBuf,
        ui_callback: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>,
    ) -> Self {
        let instance_id_str = settings.get().device_id.clone();
        let peers: Arc<Mutex<HashMap<String, (Instant, String)>>> = Arc::new(Mutex::new(HashMap::new()));
        let last_catchup: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        
        let blocked_ips_set = settings.get_blocked_ips().into_iter().collect();
        let blocked_ips: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(blocked_ips_set));

        if NETWORK_INITIALIZED.swap(true, Ordering::SeqCst) {
            println!("NetworkManager already initialized! Skipping thread spawn.");
            return Self { peers, blocked_ips, crypto: crypto.clone(), instance_id: instance_id_str.clone(), settings: settings.clone(), last_catchup, ui_callback };
        }

        // 1. UDP Discovery Broadcaster
        let crypto_for_bc = crypto.clone();
        let instance_id_bc = instance_id_str.clone();
        let db_bc = db.clone();
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
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                let mut device_name = whoami::devicename().unwrap_or_else(|_| "Unknown Device".to_string());
                #[cfg(any(target_os = "android", target_os = "ios"))]
                let mut device_name = "Mobile Device".to_string();

                if device_name.to_lowercase() == "unknown" || device_name.to_lowercase() == "localhost" || device_name.trim().is_empty() {
                    device_name = "Mobile Device".to_string();
                }
                
                // Get hash of the sync key to only discover peers with the exact same key
                if let Ok(raw_key) = crypto_for_bc.get_key() {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&raw_key);
                    let key_hash = hex::encode(hasher.finalize());
                    
                    let latest_hlc = if let Ok(db) = db_bc.lock() {
                        db.get_latest_hlc()
                    } else {
                        0
                    };
                    
                    let hlc_str = if latest_hlc > 0 { latest_hlc.to_string() } else { String::new() };
                    let msg_prefix = format!("CIPHERCLIP_DISCOVER:{}:{}:{}:{}:{}", key_hash, instance_id_bc, my_ip, device_name, hlc_str);
                    
                    let sync_mac = crypto_for_bc.generate_sync_state_mac(&msg_prefix);
                    let msg = format!("{}:{}", msg_prefix, sync_mac);                  
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
        let db_listen = db.clone();
        let last_catchup_clone = last_catchup.clone();
        let ui_callback_listen = ui_callback.clone();
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
                match socket.recv_from(&mut buf) {
                    Ok((amt, src_addr)) => {
                    // Check for our protocol prefix (CIPHERCLIP_DISCOVER:)
                    if amt > prefix_len && &buf[0..prefix_len] == prefix {
                        let msg = String::from_utf8_lossy(&buf[prefix_len..amt]);
                        let parts: Vec<&str> = msg.split(':').collect();
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
                                        let _ip = parts[2].to_string(); // ip from the message
                                        
                                        // Handle legacy format vs new format
                                        // Format: CIPHERCLIP_DISCOVER:key_hash:instance_id:ip:device_name:hlc_str:sync_mac
                                        // Note: device_name might contain colons, so we reconstruct it
                                        let is_v2 = parts.len() >= 6;
                                        let name = if is_v2 {
                                            parts[3..parts.len()-2].join(":")
                                        } else if parts.len() > 3 { 
                                            parts[3..].join(":") 
                                        } else { 
                                            "Unknown Device".to_string() 
                                        };
                                        
                                        // Use the source address from the packet as the actual peer IP
                                        // This is more reliable than the IP reported in the message,
                                        // especially on NAT/hotspot setups
                                        let actual_peer_ip = match src_addr {
                                            SocketAddr::V4(addr) => addr.ip().to_string(),
                                            SocketAddr::V6(addr) => addr.ip().to_string(),
                                        };

                                        let mut p = peers_clone.lock().unwrap();
                                        let blocked = blocked_ips_clone.lock().unwrap();
                                        
                                        // Prune inactive peers (not seen in 45 seconds)
                                        p.retain(|_, (time, _)| time.elapsed().as_secs() < 45);
                                        
                                        if !blocked.contains(&actual_peer_ip) {
                                            let is_new = !p.contains_key(&actual_peer_ip);
                                            p.insert(actual_peer_ip.clone(), (Instant::now(), name.clone()));
                                            drop(blocked);
                                            drop(p);
                                            
                                            if is_new {
                                                if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
                                                    let my_ip = get_my_ip();
                                                    
                                                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                                                    let mut device_name = whoami::devicename().unwrap_or_else(|_| "Unknown Device".to_string());
                                                    #[cfg(any(target_os = "android", target_os = "ios"))]
                                                    let mut device_name = "Mobile Device".to_string();

                                                    if device_name.to_lowercase() == "unknown" || device_name.to_lowercase() == "localhost" || device_name.trim().is_empty() {
                                                        device_name = "Mobile Device".to_string();
                                                    }
                                                    
                                                    let latest_hlc = if let Ok(db_l) = db_listen.lock() {
                                                        db_l.get_latest_hlc()
                                                    } else { 0 };
                                                    let hlc_str = if latest_hlc > 0 { latest_hlc.to_string() } else { String::new() };
                                                    let msg_prefix = format!("CIPHERCLIP_DISCOVER:{}:{}:{}:{}:{}", expected_hash, instance_id_listen, my_ip, device_name, hlc_str);
                                                    let sync_mac = crypto_for_listen.generate_sync_state_mac(&msg_prefix);
                                                    let msg = format!("{}:{}", msg_prefix, sync_mac);                  
                                                    let _ = socket.send_to(msg.as_bytes(), format!("{}:{}", actual_peer_ip, DISCOVERY_PORT));
                                                }
                                            }

                                            let db_upsert = db_listen.clone();
                                            let upsert_id = received_instance_id.to_string();
                                            let upsert_name = name.clone();
                                            std::thread::spawn(move || {
                                                if let Ok(db_l) = db_upsert.lock() {
                                                    let _ = db_l.upsert_known_peer(&upsert_id, &upsert_name);
                                                }
                                            });
                                            
                                            // Handle catch-up sync if HMAC matches
                                            if is_v2 {
                                                let hlc_str = parts[parts.len()-2];
                                                let sync_mac = parts[parts.len()-1];
                                                let msg_prefix = format!("CIPHERCLIP_DISCOVER:{}:{}:{}:{}:{}", parts[0], parts[1], parts[2], name, hlc_str);
                                                let expected_mac = crypto_for_listen.generate_sync_state_mac(&msg_prefix);
                                                
                                                if expected_mac == sync_mac {
                                                    let peer_hlc = hlc_str.parse::<i64>().unwrap_or(0);
                                                    
                                                    let should_catchup = {
                                                        let mut lc = last_catchup_clone.lock().unwrap();
                                                        let last = lc.get(&actual_peer_ip).copied();
                                                        let stale = last.map(|t| t.elapsed().as_secs() > 30).unwrap_or(true);
                                                        if stale {
                                                            lc.insert(actual_peer_ip.clone(), Instant::now());
                                                        }
                                                        stale
                                                    };
                                                    
                                                    if should_catchup {
                                                        let catchup_ip = actual_peer_ip.clone();
                                                        let crypto_catchup = crypto_for_listen.clone();
                                                        let db_catchup = db_listen.clone();
                                                        let ui_callback_catchup = ui_callback_listen.clone();
                                                        std::thread::spawn(move || {
                                                            // ── PULL: send SYNC_REQ to peer so peer sends us what we're missing ──
                                                            // This works regardless of whether the peer can TCP-connect back to us
                                                            // (important for mobile where Android may block incoming connections).
                                                            if let Ok(mut stream) = connect_tcp((catchup_ip.as_str(), TCP_PORT)) {
                                                                let map_opt = if let Ok(db_l) = db_catchup.lock() {
                                                                    let device_id = db_l.device_id.clone();
                                                                    db_l.get_all_sync_states().ok().map(|map| (device_id, map))
                                                                } else { None };

                                                                if let Some((device_id, map)) = map_opt {
                                                                    // Also push our own recent events so the peer stays up-to-date
                                                                    let mut pushed_events = if let Ok(db_l) = db_catchup.lock() {
                                                                        db_l.get_recent_events(200).unwrap_or_default()
                                                                    } else { vec![] };
                                                                    for ev in pushed_events.iter_mut() {
                                                                        if let Some(payload_bytes) = &ev.payload {
                                                                            if let Ok(dec) = crypto_catchup.decrypt(payload_bytes) {
                                                                                ev.payload = Some(dec);
                                                                            }
                                                                        }
                                                                    }

                                                                    let req_payload = serde_json::json!({
                                                                        "device_id": device_id,
                                                                        "peer_sync_state": map,
                                                                        "pushed_events": pushed_events
                                                                    }).to_string();

                                                                    if let Ok(req_enc) = crypto_catchup.encrypt(req_payload.as_bytes()) {
                                                                        let msg_type = b"SYNC_REQ";
                                                                        let mut buf = Vec::new();
                                                                        buf.push(msg_type.len() as u8);
                                                                        buf.extend_from_slice(msg_type);
                                                                        buf.extend_from_slice(&(req_enc.len() as u32).to_be_bytes());
                                                                        buf.extend_from_slice(&req_enc);
                                                                        if stream.write_all(&buf).is_ok() {
                                                                            // Read the SYNC_RES back from peer
                                                                            let mut type_len_buf = [0u8; 1];
                                                                            if stream.read_exact(&mut type_len_buf).is_ok() {
                                                                                let type_len = type_len_buf[0] as usize;
                                                                                let mut type_buf = vec![0u8; type_len];
                                                                                if stream.read_exact(&mut type_buf).is_ok() {
                                                                                    let res_type = String::from_utf8_lossy(&type_buf).to_string();
                                                                                    if res_type == "SYNC_RES" {
                                                                                        let mut len_buf = [0u8; 4];
                                                                                        if stream.read_exact(&mut len_buf).is_ok() {
                                                                                            let res_len = u32::from_be_bytes(len_buf) as usize;
                                                                                            if res_len < 50 * 1024 * 1024 {
                                                                                                let mut res_buf = vec![0u8; res_len];
                                                                                                if stream.read_exact(&mut res_buf).is_ok() {
                                                                                                    if let Ok(decrypted) = crypto_catchup.decrypt(&res_buf) {
                                                                                                        if let Ok(json_str) = String::from_utf8(decrypted) {
                                                                                                            if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                                                                                if let Some(events_arr) = res_val["events"].as_array() {
                                                                                                                    let mut events = Vec::new();
                                                                                                                    for evt in events_arr {
                                                                                                                        if let Ok(e) = serde_json::from_value::<crate::db::SyncEvent>(evt.clone()) {
                                                                                                                            events.push(e);
                                                                                                                        }
                                                                                                                    }
                                                                                                                    if !events.is_empty() {
                                                                                                                        if let Ok(db_l) = db_catchup.lock() {
                                                                                                                            let _ = db_l.apply_sync_events(events);
                                                                                                                        }
                                                                                                                        ui_callback_catchup("clipboard-update", serde_json::json!({}));
                                                                                                                    }
                                                                                                                }
                                                                                                            }
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            // ── PUSH (legacy): also try pushing our events to peer if we have more ──
                                                            // This helps when the peer can receive but not send (e.g. behind firewall)
                                                            let events = if let Ok(db) = db_catchup.lock() {
                                                                db.get_events_since_hlc(peer_hlc).unwrap_or_default()
                                                            } else {
                                                                Vec::new()
                                                            };
                                                            
                                                            if events.is_empty() { return; }
                                                            
                                                            if let Ok(mut stream) = connect_tcp((catchup_ip.as_str(), TCP_PORT)) {
                                                                let mut i = 0;
                                                                while i < events.len() {
                                                                    let batch = events[i..std::cmp::min(i + 50, events.len())].to_vec();
                                                                    let payload = serde_json::json!({
                                                                        "events": batch
                                                                    }).to_string();
                                                                    
                                                                    if let Ok(res_enc) = crypto_catchup.encrypt(payload.as_bytes()) {
                                                                        let msg_type = b"SYNC_RES";
                                                                        let mut buf = Vec::new();
                                                                        buf.push(msg_type.len() as u8);
                                                                        buf.extend_from_slice(msg_type);
                                                                        buf.extend_from_slice(&(res_enc.len() as u32).to_be_bytes());
                                                                        buf.extend_from_slice(&res_enc);
                                                                        if stream.write_all(&buf).is_err() { break; }
                                                                    }
                                                                    i += 50;
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
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
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        });

        // 3. TCP Server for incoming clips
        let crypto_for_tcp = crypto.clone();
        let crypto_tcp = crypto_for_tcp.clone();
        let blocked_ips_tcp = blocked_ips.clone();
        let peers_tcp = peers.clone();
        let ui_callback_tcp = ui_callback.clone();
        thread::spawn(move || {
            if let Ok(listener) = TcpListener::bind(("0.0.0.0", TCP_PORT)) {
                for stream_res in listener.incoming() {
                    match stream_res {
                        Ok(mut stream) => {
                            // Set timeouts on incoming connections to prevent handler threads from blocking
                            let _ = stream.set_read_timeout(Some(TCP_IO_TIMEOUT));
                            let _ = stream.set_write_timeout(Some(TCP_IO_TIMEOUT));
                            let mut src_ip = String::new();
                            if let Ok(peer_addr) = stream.peer_addr() {
                                src_ip = peer_addr.ip().to_string();
                                if blocked_ips_tcp.lock().unwrap().contains(&src_ip) {
                                    continue;
                                }
                            }
                            let crypto_c = crypto_tcp.clone();
                        let db_c = db.clone();
                        let ui_callback_c = ui_callback_tcp.clone();
                        let app_data_dir_c = app_data_dir.clone();
                        let peers_c = peers_tcp.clone();

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
                            
                            // IF BIN_RES, len_buf is num_chunks
                            if content_type == "BIN_RES" {
                                let num_chunks = u32::from_be_bytes(len_buf);
                                // Read the UUID first (we need it to save)
                                let mut uuid_len_buf = [0u8; 4];
                                if stream.read_exact(&mut uuid_len_buf).is_err() { return; }
                                let uuid_len = u32::from_be_bytes(uuid_len_buf) as usize;
                                let mut uuid_buf = vec![0u8; uuid_len];
                                if stream.read_exact(&mut uuid_buf).is_err() { return; }
                                let clip_uuid = String::from_utf8_lossy(&uuid_buf).to_string();

                                if let Ok(storage) = crate::storage::StorageManager::new(app_data_dir_c.clone()) {
                                    let _ = storage.save_chunk(&clip_uuid, &[], false); // clear file
                                    let mut downloaded_bytes = 0;
                                    for i in 0..num_chunks {
                                        let mut chunk_len_buf = [0u8; 4];
                                        if stream.read_exact(&mut chunk_len_buf).is_err() { break; }
                                        let chunk_len = u32::from_be_bytes(chunk_len_buf) as usize;
                                        if chunk_len > 10 * 1024 * 1024 { break; } // safety

                                        let mut chunk_buf = vec![0u8; chunk_len];
                                        if stream.read_exact(&mut chunk_buf).is_err() { break; }

                                        if let Ok(decrypted) = crypto_c.decrypt(&chunk_buf) {
                                            let _ = storage.save_chunk(&clip_uuid, &decrypted, true);
                                            downloaded_bytes += decrypted.len();
                                            ui_callback_c("download_progress", serde_json::json!({
                                                "uuid": clip_uuid,
                                                "progress": (i as f32 / num_chunks as f32) * 100.0,
                                                "downloaded": downloaded_bytes
                                            }));
                                        } else {
                                            break;
                                        }
                                    }
                                    ui_callback_c("download_progress", serde_json::json!({
                                        "uuid": clip_uuid,
                                        "progress": 100.0,
                                        "downloaded": downloaded_bytes
                                    }));
                                    let _ = ui_callback_c("clipboard-update", serde_json::json!({}));
                                }
                                return;
                            }

                            let payload_len = u32::from_be_bytes(len_buf) as usize;

                            if payload_len > 50 * 1024 * 1024 {
                                return;
                            }

                            let mut payload = vec![0u8; payload_len];
                            if stream.read_exact(&mut payload).is_err() {
                                return;
                            }

                            if let Ok(decrypted) = crypto_c.decrypt(&payload) {
                                if content_type == "BIN_REQ" {
                                    if let Ok(uuid) = String::from_utf8(decrypted) {
                                        if let Ok(storage) = crate::storage::StorageManager::new(app_data_dir_c.clone()) {
                                            let enc_path = storage.get_encrypted_attachment_path(&uuid);
                                            let path = storage.get_attachment_path(&uuid);
                                            let legacy = storage.get_legacy_attachment_path(&uuid);
                                            
                                            let bytes = if enc_path.exists() {
                                                if let Ok(enc_bytes) = std::fs::read(&enc_path) {
                                                    crypto_c.decrypt(&enc_bytes).unwrap_or_default()
                                                } else { Vec::new() }
                                            } else if path.exists() {
                                                std::fs::read(&path).unwrap_or_default()
                                            } else if legacy.exists() {
                                                std::fs::read(&legacy).unwrap_or_default()
                                            } else {
                                                Vec::new()
                                            };

                                            if !bytes.is_empty() {
                                                let file_size = bytes.len() as u64;
                                                let num_chunks = ((file_size + 65535) / 65536) as u32;

                                                // Send BIN_RES header
                                                let res_type = b"BIN_RES";
                                                let mut header = Vec::new();
                                                header.push(res_type.len() as u8);
                                                header.extend_from_slice(res_type);
                                                header.extend_from_slice(&num_chunks.to_be_bytes());
                                                
                                                // Send UUID so receiver knows which file this is
                                                let uuid_bytes = uuid.as_bytes();
                                                header.extend_from_slice(&(uuid_bytes.len() as u32).to_be_bytes());
                                                header.extend_from_slice(uuid_bytes);
                                                
                                                let mut stream_clone = stream.try_clone().unwrap();
                                                if stream_clone.write_all(&header).is_ok() {
                                                    let crypto_clone = crypto_c.clone();
                                                    for chunk in bytes.chunks(65536) {
                                                        if let Ok(encrypted_chunk) = crypto_clone.encrypt(chunk) {
                                                            let mut chunk_packet = Vec::new();
                                                            chunk_packet.extend_from_slice(&(encrypted_chunk.len() as u32).to_be_bytes());
                                                            chunk_packet.extend_from_slice(&encrypted_chunk);
                                                            if stream_clone.write_all(&chunk_packet).is_err() {
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    return;
                                }

                                if content_type == "SYNC_REQ" {
                                    if let Ok(mut p) = peers_c.lock() {
                                        if let Some((time, _name)) = p.get_mut(&src_ip) {
                                            *time = std::time::Instant::now();
                                        }
                                    }
                                    if let Ok(json_str) = String::from_utf8(decrypted) {
                                        if let Ok(req_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                            if let (Some(_device_id), Some(peer_sync_state_obj)) = (req_val["device_id"].as_str(), req_val["peer_sync_state"].as_object()) {
                                                
                                                if let Some(pushed_arr) = req_val["pushed_events"].as_array() {
                                                    let mut pushed_events = Vec::new();
                                                    for evt in pushed_arr {
                                                        match serde_json::from_value::<crate::db::SyncEvent>(evt.clone()) {
                                                            Ok(mut e) => {
                                                                if let Some(payload_bytes) = &e.payload {
                                                                    if let Ok(enc) = crypto_c.encrypt(payload_bytes) {
                                                                        e.payload = Some(enc);
                                                                    }
                                                                }
                                                                pushed_events.push(e);
                                                            }
                                                            Err(err) => println!("Failed to parse SyncEvent in SYNC_REQ: {:?}", err),
                                                        }
                                                    }
                                                    if !pushed_events.is_empty() {
                                                        let events_clone = pushed_events.clone();
                                                        if let Ok(db_lock) = db_c.lock() {
                                                            match db_lock.apply_sync_events(pushed_events) {
                                                              Ok(_) => println!("Successfully applied SYNC_REQ pushed events"),
                                                              Err(e) => println!("Error applying SYNC_REQ pushed events: {:?}", e),
                                                          }
                                                            // Tell UI the attachment is downloading BEFORE emitting clipboard-update, ONLY if it actually needs downloading
                                                            for e in &events_clone {
                                                                if e.has_attachment.unwrap_or(false) {
                                                                    let raw_path = e.attachment_path.clone().unwrap_or_default();
                                                                    let extracted = std::path::Path::new(&raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                                                                    let uuid = if extracted.is_empty() { e.clip_uuid.clone() } else { extracted };
                                                                    let path = app_data_dir_c.join("attachments").join(format!("{}.png", uuid));
                                                                    let legacy_path = app_data_dir_c.join("attachments").join(format!("{}.bin", uuid));
                                                                    
                                                                    if !path.exists() && !legacy_path.exists() {
                                                                        let _ = ui_callback_c("download_progress", serde_json::json!({
                                                                            "uuid": uuid,
                                                                            "progress": 0,
                                                                            "downloaded": 0
                                                                        }));
                                                                    }
                                                                }
                                                            }
                                                            let _ = ui_callback_c("clipboard-update", serde_json::json!({}));
                                                        }
                                                        for e in events_clone {
                                                            if e.has_attachment.unwrap_or(false) {
                                                                let peer_ip = src_ip.to_string();
                                                                let raw_path = e.attachment_path.clone().unwrap_or_default();
                                                                let extracted = std::path::Path::new(&raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                                                                let uuid = if extracted.is_empty() { e.clip_uuid.clone() } else { extracted };
                                                                let c_crypto = crypto_c.clone();
                                                                let c_dir = app_data_dir_c.clone();
                                                                let c_cb = ui_callback_c.clone();
                                                                
                                                                let path = c_dir.join("attachments").join(format!("{}.png", uuid));
                                                                let legacy_path = c_dir.join("attachments").join(format!("{}.bin", uuid));
                                                                if !path.exists() && !legacy_path.exists() {
                                                                    std::thread::spawn(move || {
                                                                        let _ = crate::network::download_attachment(&peer_ip, &uuid, c_crypto, c_dir, c_cb);
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                let mut peer_clocks = std::collections::HashMap::new();
                                                for (k, v) in peer_sync_state_obj {
                                                    if let Some(c) = v.as_i64() {
                                                        peer_clocks.insert(k.clone(), c);
                                                    }
                                                }
                                                let mut missing_events = if let Ok(db_lock) = db_c.lock() {
                                                    db_lock.get_missing_events(&peer_clocks, 50).unwrap_or_default()
                                                } else { vec![] };
                                                for ev in missing_events.iter_mut() {
                                                    if let Some(payload_bytes) = &ev.payload {
                                                        if let Ok(dec) = crypto_c.decrypt(payload_bytes) {
                                                            ev.payload = Some(dec);
                                                        }
                                                    }
                                                }

                                                // Always send SYNC_RES (even empty) so the client
                                                // doesn't hang waiting for a response.
                                                let res_payload = serde_json::json!({
                                                    "events": missing_events
                                                }).to_string();
                                                
                                                if let Ok(res_enc) = crypto_c.encrypt(res_payload.as_bytes()) {
                                                    let msg_type = b"SYNC_RES";
                                                    let mut buf = Vec::new();
                                                    buf.push(msg_type.len() as u8);
                                                    buf.extend_from_slice(msg_type);
                                                    buf.extend_from_slice(&(res_enc.len() as u32).to_be_bytes());
                                                    buf.extend_from_slice(&res_enc);
                                                    let mut write_stream = stream.try_clone().expect("clone stream failed");
                                                    let _ = std::io::Write::write_all(&mut write_stream, &buf);
                                                }
                                            }
                                        }
                                    }
                                    return;
                                } else if content_type == "SYNC_RES" {
                                    if let Ok(mut p) = peers_c.lock() {
                                        if let Some((time, _name)) = p.get_mut(&src_ip) {
                                            *time = std::time::Instant::now();
                                        }
                                    }
                                    if let Ok(json_str) = String::from_utf8(decrypted) {
                                        if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                            if let Some(events_arr) = res_val["events"].as_array() {
                                                let mut events = Vec::new();
                                                for evt in events_arr {
                                                    match serde_json::from_value::<crate::db::SyncEvent>(evt.clone()) {
                                                        Ok(mut e) => {
                                                            if let Some(payload_bytes) = &e.payload {
                                                                if let Ok(enc) = crypto_c.encrypt(payload_bytes) {
                                                                    e.payload = Some(enc);
                                                                }
                                                            }
                                                            events.push(e);
                                                        }
                                                        Err(err) => println!("Failed to parse SyncEvent in SYNC_RES: {:?}", err),
                                                    }
                                                }
                                                let events_clone = events.clone();
                                                if let Ok(db_lock) = db_c.lock() {
                                                      match db_lock.apply_sync_events(events) {
                                                          Ok(_) => println!("Successfully applied SYNC_RES events"),
                                                          Err(e) => println!("Error applying SYNC_RES events: {:?}", e),
                                                      }
                                                      // Tell UI the attachment is downloading BEFORE emitting clipboard-update, ONLY if it actually needs downloading
                                                      for e in &events_clone {
                                                          if e.has_attachment.unwrap_or(false) {
                                                              let raw_path = e.attachment_path.clone().unwrap_or_default();
                                                              let extracted = std::path::Path::new(&raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                                                              let uuid = if extracted.is_empty() { e.clip_uuid.clone() } else { extracted };
                                                              let path = app_data_dir_c.join("attachments").join(format!("{}.png", uuid));
                                                              let legacy_path = app_data_dir_c.join("attachments").join(format!("{}.bin", uuid));
                                                              
                                                              if !path.exists() && !legacy_path.exists() {
                                                                  let _ = ui_callback_c("download_progress", serde_json::json!({
                                                                      "uuid": uuid,
                                                                      "progress": 0,
                                                                      "downloaded": 0
                                                                  }));
                                                              }
                                                          }
                                                      }
                                                    let _ = ui_callback_c("clipboard-update", serde_json::json!({}));
                                                }
                                                for e in events_clone {
                                                    if e.has_attachment.unwrap_or(false) {
                                                        let peer_ip = src_ip.to_string();
                                                        let raw_path = e.attachment_path.clone().unwrap_or_default();
                                                        let extracted = std::path::Path::new(&raw_path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                                                        let uuid = if extracted.is_empty() { e.clip_uuid.clone() } else { extracted };
                                                        let c_crypto = crypto_c.clone();
                                                        let c_dir = app_data_dir_c.clone();
                                                        let c_cb = ui_callback_c.clone();
                                                        
                                                        let path = c_dir.join("attachments").join(format!("{}.png", uuid));
                                                        let legacy_path = c_dir.join("attachments").join(format!("{}.bin", uuid));
                                                        if !path.exists() && !legacy_path.exists() {
                                                            std::thread::spawn(move || {
                                                                let _ = crate::network::download_attachment(&peer_ip, &uuid, c_crypto, c_dir, c_cb);
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    return;
                                }

                                // Handle Legacy EVENT (if needed, but we don't send it anymore? We will just keep it to avoid crashing)
                                if content_type == "EVENT" {
                                    // Just ignore old EVENT protocol as we now use Sync Events via push/pull
                                    return;
                                }
                                
                                // Actually, for immediate push (push_clip), we could just rely on the sync protocol
                                // by immediately generating a Sync Event in insert_clip, but push_clip expects legacy payload.
                                // If content_type is TEXT or IMAGE (pushing clips)
                                if let Ok(db_lock) = db_c.lock() {
                                    // It decrypted cleanly. Save it using Event Sourcing via insert_clip.
                                    let _ = db_lock.insert_clip(&content_type, &payload, 100, false, None);
                                    let _ = ui_callback_c("clipboard-update", serde_json::json!({}));
                                }
                            }
                        });
                    }
                    Err(e) => {
                        log::error!("TCP accept error: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                }
            }
        });

        Self { peers, blocked_ips, crypto, instance_id: instance_id_str, settings, last_catchup, ui_callback }
    }

    
    pub fn trigger_sync(&self, db: std::sync::Arc<std::sync::Mutex<crate::db::Database>>) {
        let peers: Vec<String> = self.peers.lock().unwrap().keys().cloned().collect();
        for peer_ip in peers {
            let db_clone = db.clone();
            let crypto_clone = self.crypto.clone();
            let ui_callback_clone = self.ui_callback.clone();
            std::thread::spawn(move || {
                if let Ok(mut stream) = connect_tcp((peer_ip.as_str(), TCP_PORT)) {
                    let map_opt = if let Ok(db_l) = db_clone.lock() {
                        let device_id = db_l.device_id.clone();
                        db_l.get_all_sync_states().ok().map(|map| (device_id, map))
                    } else { None };

                    if let Some((device_id, map)) = map_opt {
                        let mut pushed_events = if let Ok(db_l) = db_clone.lock() {
                            db_l.get_recent_events(200).unwrap_or_default()
                        } else { vec![] };
                        for ev in pushed_events.iter_mut() {
                            if let Some(payload_bytes) = &ev.payload {
                                if let Ok(dec) = crypto_clone.decrypt(payload_bytes) {
                                    ev.payload = Some(dec);
                                }
                            }
                        }
                        let payload = serde_json::json!({
                            "device_id": device_id,
                            "peer_sync_state": map,
                            "pushed_events": pushed_events
                        }).to_string();
                        if let Ok(encrypted) = crypto_clone.encrypt(payload.as_bytes()) {
                            let msg_type = b"SYNC_REQ";
                            let mut buf = Vec::new();
                            buf.push(msg_type.len() as u8);
                            buf.extend_from_slice(msg_type);
                            buf.extend_from_slice(&(encrypted.len() as u32).to_be_bytes());
                            buf.extend_from_slice(&encrypted);
                            if std::io::Write::write_all(&mut stream, &buf).is_ok() {
                                // Read the SYNC_RES that the peer sends back on this same connection
                                let mut type_len_buf = [0u8; 1];
                                if stream.read_exact(&mut type_len_buf).is_ok() {
                                    let type_len = type_len_buf[0] as usize;
                                    let mut type_buf = vec![0u8; type_len];
                                    if stream.read_exact(&mut type_buf).is_ok() {
                                        let res_type = String::from_utf8_lossy(&type_buf).to_string();
                                        if res_type == "SYNC_RES" {
                                            let mut len_buf = [0u8; 4];
                                            if stream.read_exact(&mut len_buf).is_ok() {
                                                let res_len = u32::from_be_bytes(len_buf) as usize;
                                                if res_len < 50 * 1024 * 1024 {
                                                    let mut res_buf = vec![0u8; res_len];
                                                    if stream.read_exact(&mut res_buf).is_ok() {
                                                        if let Ok(decrypted) = crypto_clone.decrypt(&res_buf) {
                                                            if let Ok(json_str) = String::from_utf8(decrypted) {
                                                                if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                                    if let Some(events_arr) = res_val["events"].as_array() {
                                                                        let mut events = Vec::new();
                                                                        for evt in events_arr {
                                                                            if let Ok(mut e) = serde_json::from_value::<crate::db::SyncEvent>(evt.clone()) {
                                                                                if let Some(payload_bytes) = &e.payload {
                                                                                    if let Ok(enc) = crypto_clone.encrypt(payload_bytes) {
                                                                                        e.payload = Some(enc);
                                                                                    }
                                                                                }
                                                                                events.push(e);
                                                                            }
                                                                        }
                                                                        if !events.is_empty() {
                                                                            if let Ok(db_l) = db_clone.lock() {
                                                                                let _ = db_l.apply_sync_events(events);
                                                                            }
                                                                            ui_callback_clone("clipboard-update", serde_json::json!({}));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    }
pub fn get_connected_peers(&self) -> Vec<PeerInfo> {
        let mut p = self.peers.lock().unwrap();
        p.retain(|_, (time, _)| time.elapsed().as_secs() < 45);
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


pub fn download_attachment(peer_ip: &str, uuid: &str, crypto: std::sync::Arc<crate::crypto::CryptoState>, app_data_dir: PathBuf, ui_callback: std::sync::Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>) -> Result<(), String> {
    if let Ok(mut stream) = connect_tcp((peer_ip, TCP_PORT)) {
        let req_str = uuid.to_string();
        let req_bytes = req_str.as_bytes();
        
        let req_type = b"BIN_REQ";
        let mut header = Vec::new();
        header.push(req_type.len() as u8);
        header.extend_from_slice(req_type);
        header.extend_from_slice(&(req_bytes.len() as u32).to_be_bytes());
        header.extend_from_slice(req_bytes);

        // We don't encrypt the BIN_REQ payload currently, we just send it as raw bytes for simplicity, 
        // wait! The server expects the payload to be encrypted by `crypto_c.decrypt(&payload)`.
        // We must fetch crypto! We can't access it easily here without passing it.
        // Actually, we can fetch it from AppState!
        if let Ok(encrypted) = crypto.encrypt(req_bytes) {
            let mut header = Vec::new();
            header.push(req_type.len() as u8);
            header.extend_from_slice(req_type);
            header.extend_from_slice(&(encrypted.len() as u32).to_be_bytes());
            header.extend_from_slice(&encrypted);

            if stream.write_all(&header).is_ok() {
                // The response is processed by the main TCP listener loop because we just wrote to stream!
                // Wait, no. We initiated the connection, we are the client! The server writes back to this stream.
                // We need to read from THIS stream!
                let mut type_len_buf = [0u8; 1];
                if stream.read_exact(&mut type_len_buf).is_ok() {
                    let type_len = type_len_buf[0] as usize;
                    let mut type_buf = vec![0u8; type_len];
                    if stream.read_exact(&mut type_buf).is_ok() {
                        let content_type = String::from_utf8_lossy(&type_buf).to_string();
                        if content_type == "BIN_RES" {
                            let mut len_buf = [0u8; 4];
                            if stream.read_exact(&mut len_buf).is_ok() {
                                let num_chunks = u32::from_be_bytes(len_buf);
                                let mut uuid_len_buf = [0u8; 4];
                                if stream.read_exact(&mut uuid_len_buf).is_ok() {
                                    let uuid_len = u32::from_be_bytes(uuid_len_buf) as usize;
                                    let mut uuid_buf = vec![0u8; uuid_len];
                                    if stream.read_exact(&mut uuid_buf).is_ok() {
                                        let clip_uuid = String::from_utf8_lossy(&uuid_buf).to_string();
                                        if let Ok(storage) = crate::storage::StorageManager::new(app_data_dir.clone()) {
                                            let mut file_data = Vec::new();
                                            let mut downloaded_bytes = 0;
                                            for i in 0..num_chunks {
                                                let mut chunk_len_buf = [0u8; 4];
                                                if stream.read_exact(&mut chunk_len_buf).is_err() { break; }
                                                let chunk_len = u32::from_be_bytes(chunk_len_buf) as usize;
                                                if chunk_len > 10 * 1024 * 1024 { break; }
                                                let mut chunk_buf = vec![0u8; chunk_len];
                                                if stream.read_exact(&mut chunk_buf).is_err() { break; }

                                                if let Ok(decrypted) = crypto.decrypt(&chunk_buf) {
                                                    file_data.extend_from_slice(&decrypted);
                                                    downloaded_bytes += decrypted.len();
                                                    ui_callback("download_progress", serde_json::json!({ "uuid": clip_uuid.clone(), "progress": if num_chunks > 0 { (i as f64 / num_chunks as f64 * 100.0) as u32 } else { 100 }, "downloaded": downloaded_bytes }));
                                                } else { break; }
                                            }
                                            let enc_path = storage.get_encrypted_attachment_path(&clip_uuid);
                                            if let Ok(encrypted_file) = crypto.encrypt(&file_data) {
                                                let _ = std::fs::write(&enc_path, &encrypted_file);
                                            }
                                            ui_callback("download_progress", serde_json::json!({ "uuid": clip_uuid.clone(), "progress": 100, "downloaded": downloaded_bytes }));
                                            ui_callback("clipboard-update", serde_json::json!({}));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
