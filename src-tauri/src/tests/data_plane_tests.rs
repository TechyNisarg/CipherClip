use tempfile::tempdir;
use std::fs::File;
use std::io::{Write, Read};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use tokio::time::sleep;

use crate::crypto::CryptoState;
use crate::db::Database;
use crate::settings::SettingsManager;
use crate::network::{NetworkManager, download_attachment};
use crate::storage::StorageManager;

fn get_ui_callback(test_name: &'static str) -> Arc<dyn Fn(&str, serde_json::Value) + Send + Sync> {
    let visual = std::env::var("CIPHERCLIP_VISUAL_TEST").unwrap_or_else(|_| "0".to_string()) == "1";
    let pb = if visual {
        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}% ({msg})")
            .unwrap()
            .progress_chars("#>-"));
        Some(Arc::new(Mutex::new(pb)))
    } else {
        None
    };

    let test_name_string = test_name.to_string();

    Arc::new(move |event, payload| {
        if event == "download_progress" {
            if let Some(pct) = payload.get("percentage").and_then(|p| p.as_u64()) {
                if let Some(pb_arc) = &pb {
                    if let Ok(pb_lock) = pb_arc.lock() {
                        pb_lock.set_position(pct);
                        pb_lock.set_message(test_name_string.clone());
                        if pct == 100 {
                            pb_lock.finish_with_message("Done!");
                        }
                    }
                }
            }
        }
    })
}

fn create_dummy_file(dir: &std::path::Path, uuid: &str, size_mb: usize) -> Vec<u8> {
    let storage = StorageManager::new(dir.to_path_buf()).unwrap();
    let path = storage.get_attachment_path(uuid);
    let mut file = File::create(&path).unwrap();
    
    let mut hasher = Sha256::new();
    let chunk_size = 1024 * 1024; // 1 MB
    let chunk: Vec<u8> = (0..chunk_size).map(|i| (i % 256) as u8).collect();
    
    for _ in 0..size_mb {
        file.write_all(&chunk).unwrap();
        hasher.update(&chunk);
    }
    
    hasher.finalize().to_vec()
}

fn calculate_file_hash(path: &std::path::Path) -> Vec<u8> {
    let mut file = File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 { break; }
        hasher.update(&buffer[..n]);
    }
    hasher.finalize().to_vec()
}

fn setup_network_manager(app_dir: &std::path::Path, crypto: Arc<CryptoState>) -> Arc<NetworkManager> {
    let db = Arc::new(Mutex::new(Database::new(app_dir.to_path_buf(), "test_device".to_string()).unwrap()));
    let settings = Arc::new(SettingsManager::new(app_dir.to_path_buf()));
    let ui_callback: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync> = Arc::new(|_, _| {});
    
    Arc::new(NetworkManager::new(crypto, db, settings, app_dir.to_path_buf(), ui_callback))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_data_plane_integration() {
    let sender_dir = tempdir().unwrap();
    let shared_crypto = Arc::new(CryptoState::new(&sender_dir.path().to_path_buf()).unwrap());
    let _sender_network = setup_network_manager(sender_dir.path(), shared_crypto.clone());
    sleep(Duration::from_millis(1000)).await;
    
    // --- Scenario 1: The Streaming Memory (OOM) Test ---
    println!("Running Scenario 1: OOM Test (500MB)");
    let uuid_1 = "oom-test";
    let original_hash_1 = create_dummy_file(sender_dir.path(), uuid_1, 500);
    
    let receiver_dir_1 = tempdir().unwrap();
    let receiver_crypto = shared_crypto.clone();
    
    let result = download_attachment(
        "127.0.0.1", uuid_1, receiver_crypto.clone(), receiver_dir_1.path().to_path_buf(), get_ui_callback("OOM Test")
    );
    assert!(result.is_ok(), "Scenario 1 failed");
    let storage_1 = StorageManager::new(receiver_dir_1.path().to_path_buf()).unwrap();
    assert_eq!(original_hash_1, calculate_file_hash(&storage_1.get_attachment_path(uuid_1)));
    
    // --- Scenario 2: The Interruption Test ---
    println!("Running Scenario 2: Interruption Test");
    let uuid_2 = "interrupt-test";
    create_dummy_file(sender_dir.path(), uuid_2, 100);
    
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", 45556)).unwrap();
    let req_type = b"BIN_REQ";
    let mut header = Vec::new();
    header.push(req_type.len() as u8);
    header.extend_from_slice(req_type);
    let encrypted_uuid = shared_crypto.encrypt(uuid_2.as_bytes()).unwrap();
    header.extend_from_slice(&(encrypted_uuid.len() as u32).to_be_bytes());
    header.extend_from_slice(&encrypted_uuid);
    stream.write_all(&header).unwrap();
    
    let mut res_type_len = [0u8; 1];
    stream.read_exact(&mut res_type_len).unwrap();
    let mut res_type = vec![0u8; res_type_len[0] as usize];
    stream.read_exact(&mut res_type).unwrap();
    assert_eq!(&res_type, b"BIN_RES");
    
    let mut num_chunks_buf = [0u8; 4];
    stream.read_exact(&mut num_chunks_buf).unwrap();
    let num_chunks = u32::from_be_bytes(num_chunks_buf);
    assert!(num_chunks > 10);
    
    for _ in 0..5 {
        let mut chunk_len_buf = [0u8; 4];
        stream.read_exact(&mut chunk_len_buf).unwrap();
        let chunk_len = u32::from_be_bytes(chunk_len_buf) as usize;
        let mut chunk_buf = vec![0u8; chunk_len];
        stream.read_exact(&mut chunk_buf).unwrap();
    }
    drop(stream);
    sleep(Duration::from_millis(500)).await;
    
    // --- Scenario 3: The Concurrent Sync Test ---
    println!("Running Scenario 3: Concurrent Sync Test (5x50MB)");
    let mut handles = vec![];
    for i in 0..5 {
        let s_dir_path = sender_dir.path().to_path_buf();
        let uuid_n = format!("concurrent-{}", i);
        let hash_n = create_dummy_file(&s_dir_path, &uuid_n, 50);
        
        let rcrypto = receiver_crypto.clone();
        
        handles.push(tokio::task::spawn_blocking(move || {
            let rdir = tempfile::tempdir().unwrap();
            
            // Generate a unique static name or use string formatting inside the closure
            let test_name = Box::leak(format!("Concurrent {}", i).into_boxed_str());
            
            let result = download_attachment(
                "127.0.0.1", &uuid_n, rcrypto, rdir.path().to_path_buf(), get_ui_callback(test_name)
            );
            assert!(result.is_ok(), "Concurrent {} failed", i);
            let storage = StorageManager::new(rdir.path().to_path_buf()).unwrap();
            let dhash = calculate_file_hash(&storage.get_attachment_path(&uuid_n));
            assert_eq!(hash_n, dhash);
        }));
    }
    
    for h in handles {
        h.await.unwrap();
    }
    
    println!("All scenarios passed!");
}
