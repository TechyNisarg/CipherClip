use crate::crypto::CryptoState;
use crate::db::Database;
use local_ip_address::local_ip;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::Emitter;

const DISCOVERY_PORT: u16 = 45555;
const TCP_PORT: u16 = 45556;
const MAGIC_WORD: &[u8] = b"CIPHERCLIP_DISCOVER";

pub struct NetworkManager {
    peers: Arc<Mutex<HashSet<String>>>,
}

impl NetworkManager {
    pub fn new(
        app_handle: tauri::AppHandle,
        crypto: Arc<CryptoState>,
        db: Arc<Mutex<Database>>,
    ) -> Self {
        let peers = Arc::new(Mutex::new(HashSet::new()));

        // 1. UDP Discovery Broadcaster
        thread::spawn(move || loop {
            if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
                let _ = socket.set_broadcast(true);
                let my_ip = local_ip().unwrap_or_else(|_| "127.0.0.1".parse().unwrap());

                let msg = format!("CIPHERCLIP_DISCOVER:{}", my_ip);
                let _ = socket.send_to(msg.as_bytes(), ("255.255.255.255", DISCOVERY_PORT));
            }
            thread::sleep(std::time::Duration::from_secs(5));
        });

        // 2. UDP Discovery Listener
        let peers_clone = peers.clone();
        thread::spawn(move || {
            if let Ok(socket) = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
                let mut buf = [0; 1024];
                loop {
                    if let Ok((amt, _src)) = socket.recv_from(&mut buf) {
                        if amt > 19 && &buf[0..19] == MAGIC_WORD {
                            let msg = String::from_utf8_lossy(&buf[19..amt]);
                            let ip = msg.trim_start_matches(':');

                            let my_ip = local_ip()
                                .unwrap_or_else(|_| "127.0.0.1".parse().unwrap())
                                .to_string();
                            if ip != my_ip {
                                let mut p = peers_clone.lock().unwrap();
                                p.insert(ip.to_string());
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
                            if crypto_c.decrypt(&payload).is_ok() {
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

        Self { peers }
    }

    pub fn push_clip(&self, content_type: &str, encrypted_payload: &[u8]) {
        let peers = self.peers.lock().unwrap().clone();
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
}
