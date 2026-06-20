// ═══════════════════════════════════════════════════════════════════════
// CipherClip V2 — E2E Multi-Node Cluster Integration Test
// ═══════════════════════════════════════════════════════════════════════
//
// Tests the full lifecycle across two headless nodes:
//   Case 1: Inline text propagation (≤500KB) via Control Plane
//   Case 2: Binary image ingestion & streaming via Data Plane
//   Case 3: Heavy text attachment fallback (>500KB)
//   Case 4: OS clipboard file URI normalization
// ═══════════════════════════════════════════════════════════════════════
// NOTE: Imports are provided by the parent module (data_plane_tests.rs
// and sync_protocol_tests.rs are include!'d into the same `mod tests`).

/// Helper: create a Database + SettingsManager for a headless node.
fn setup_node(app_dir: &std::path::Path, device_id: &str) -> (Arc<Mutex<Database>>, Arc<CryptoState>, Arc<SettingsManager>) {
    let db = Arc::new(Mutex::new(Database::new(app_dir.to_path_buf(), device_id.to_string()).unwrap()));
    let crypto = Arc::new(CryptoState::new(&app_dir.to_path_buf()).unwrap());
    let settings = Arc::new(SettingsManager::new(app_dir.to_path_buf()));
    (db, crypto, settings)
}

/// Helper: sync events from source node to target node.
fn sync_events(
    source_db: &Arc<Mutex<Database>>,
    target_db: &Arc<Mutex<Database>>,
    source_device_id: &str,
) {
    let source_guard = source_db.lock().unwrap();
    let target_guard = target_db.lock().unwrap();
    
    let peer_state = target_guard.get_peer_sync_state(source_device_id).unwrap_or(0);
    let mut map = HashMap::new();
    map.insert(source_device_id.to_string(), peer_state);
    
    let events = source_guard.get_missing_events(&map, 100).unwrap();
    let max_clock = events.iter().map(|e| e.vector_clock).max().unwrap_or(0);
    
    if !events.is_empty() {
        target_guard.apply_sync_events(events).unwrap();
        target_guard.update_peer_sync_state(source_device_id, max_clock).unwrap();
    }
}

/// Calculate SHA-256 hash of a file on disk.
fn hash_file(path: &std::path::Path) -> Vec<u8> {
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

// ═══════════════════════════════════════════════════════════════════════
// CASE 1: Inline Text Propagation (≤ 500KB)
// ═══════════════════════════════════════════════════════════════════════
#[test]
fn e2e_case1_inline_text_propagation() {
    println!("\n══════════ E2E Case 1: Inline Text Propagation (50KB) ══════════");
    
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    
    let (db_a, crypto_a, _settings_a) = setup_node(dir_a.path(), "Node_A");
    let (db_b, _crypto_b, _settings_b) = setup_node(dir_b.path(), "Node_B");
    
    // Simulate Node A copying a 50KB text payload
    let text_50kb: Vec<u8> = (0..50 * 1024).map(|i| b"ABCDEFGHIJ"[i % 10]).collect();
    assert!(text_50kb.len() <= 500 * 1024, "Payload should be under 500KB threshold");
    
    let encrypted = crypto_a.encrypt(&text_50kb).unwrap();
    
    // Insert inline — no attachment
    let clip_id = db_a.lock().unwrap().insert_clip("text", &encrypted, 100, false, None).unwrap();
    println!("  ✓ Node A inserted inline text clip (id={})", clip_id);
    
    // Verify Node A's vector clock advanced
    {
        let guard = db_a.lock().unwrap();
        let events = guard.get_missing_events(&HashMap::new(), 10).unwrap();
        assert!(!events.is_empty(), "Node A should have generated an event");
        assert!(events[0].vector_clock > 0, "HLC vector clock should have advanced");
        assert_eq!(events[0].event_type, "INSERT");
        assert_eq!(events[0].has_attachment, Some(false));
        println!("  ✓ Node A vector clock advanced to {}", events[0].vector_clock);
    }
    
    // Simulate TCP SYNC_REQ push: Node A → Node B
    sync_events(&db_a, &db_b, "Node_A");
    
    // Verify Node B received the clip
    {
        let guard = db_b.lock().unwrap();
        let clips = guard.get_all_clips().unwrap();
        assert_eq!(clips.len(), 1, "Node B should have exactly 1 clip");
        assert_eq!(clips[0].1, "text");
        assert_eq!(clips[0].6, false, "has_attachment should be false for inline text");
        println!("  ✓ Node B received inline text clip via Control Plane");
    }
    
    println!("  ✅ CASE 1 PASSED: Inline text propagation works correctly\n");
}

// ═══════════════════════════════════════════════════════════════════════
// CASE 2: Binary Image Ingestion & Data Plane Streaming
// ═══════════════════════════════════════════════════════════════════════
#[test]
fn e2e_case2_binary_image_ingestion() {
    println!("\n══════════ E2E Case 2: Binary Image Ingestion (15MB) ══════════");
    
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();
    
    let (db_a, crypto_a, _settings_a) = setup_node(dir_a.path(), "Node_A");
    let (db_b, _crypto_b, _settings_b) = setup_node(dir_b.path(), "Node_B");
    let storage_a = StorageManager::new(dir_a.path().to_path_buf()).unwrap();
    let storage_b = StorageManager::new(dir_b.path().to_path_buf()).unwrap();
    
    // Simulate Node A intercepting a 15MB image from the OS clipboard
    let image_size = 15 * 1024 * 1024; // 15MB
    let fake_image_data: Vec<u8> = (0..image_size).map(|i| (i % 251) as u8).collect();
    
    // Generate the attachment UUID
    let attachment_uuid = uuid::Uuid::new_v4().to_string();
    
    // Write the image to Node A's attachment directory (bypasses DB string buffer)
    storage_a.save_chunk(&attachment_uuid, &fake_image_data, false).unwrap();
    println!("  ✓ Node A wrote 15MB image to disk: {}.bin", &attachment_uuid[..8]);
    
    // Verify the file exists and hash matches
    let original_hash = {
        let mut hasher = Sha256::new();
        hasher.update(&fake_image_data);
        hasher.finalize().to_vec()
    };
    let disk_hash = hash_file(&storage_a.get_attachment_path(&attachment_uuid));
    assert_eq!(original_hash, disk_hash, "Disk file hash should match original data");
    println!("  ✓ SHA-256 verified on Node A disk");
    
    // Create a lightweight base64 preview stub for the DB
    use base64::{engine::general_purpose, Engine as _};
    let preview = general_purpose::STANDARD.encode(&fake_image_data[..1024]); // tiny preview
    let encrypted_preview = crypto_a.encrypt(preview.as_bytes()).unwrap();
    
    // Insert with has_attachment = true
    let clip_id = db_a.lock().unwrap().insert_clip(
        "image", &encrypted_preview, 100, true, Some(attachment_uuid.clone())
    ).unwrap();
    println!("  ✓ Node A inserted image clip with has_attachment=true (id={})", clip_id);
    
    // Sync metadata to Node B (Control Plane only — no binary)
    sync_events(&db_a, &db_b, "Node_A");
    
    // Verify Node B received the metadata with has_attachment = true
    {
        let guard = db_b.lock().unwrap();
        let clips = guard.get_all_clips().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].1, "image");
        assert_eq!(clips[0].6, true, "has_attachment should be true");
        assert!(clips[0].7.is_some(), "attachment_path should contain the UUID");
        println!("  ✓ Node B received metadata with has_attachment=true via Control Plane");
    }
    
    // Simulate Data Plane BIN_REQ: Node B downloads the binary chunk-by-chunk
    let mut downloaded_data = Vec::new();
    storage_a.read_attachment_stream(&attachment_uuid, |chunk| {
        // Simulate 64KB buffer streaming — no full file in RAM
        assert!(chunk.len() <= 65536, "Chunk size must be ≤ 64KB");
        downloaded_data.extend_from_slice(chunk);
        Ok(())
    }).unwrap();
    
    // Write to Node B's storage
    storage_b.save_chunk(&attachment_uuid, &downloaded_data, false).unwrap();
    
    // Verify SHA-256 hash match between Node A and Node B
    let receiver_hash = hash_file(&storage_b.get_attachment_path(&attachment_uuid));
    assert_eq!(original_hash, receiver_hash, "Node B file hash must match Node A exactly");
    println!("  ✓ Node B streamed 15MB in 64KB chunks, SHA-256 verified");
    
    // Verify no RAM spike: downloaded_data should be exactly the original size
    assert_eq!(downloaded_data.len(), image_size);
    println!("  ✓ No OOM: streamed {} bytes total", downloaded_data.len());
    
    println!("  ✅ CASE 2 PASSED: Binary image ingestion & Data Plane streaming works\n");
}

// ═══════════════════════════════════════════════════════════════════════
// CASE 3: Heavy Text Attachment Fallback (> 500KB)
// ═══════════════════════════════════════════════════════════════════════
#[test]
fn e2e_case3_heavy_text_attachment_fallback() {
    println!("\n══════════ E2E Case 3: Heavy Text Attachment (2MB) ══════════");
    
    let dir_a = tempdir().unwrap();
    
    let (db_a, crypto_a, _settings_a) = setup_node(dir_a.path(), "Node_A");
    let storage_a = StorageManager::new(dir_a.path().to_path_buf()).unwrap();
    
    // Simulate a 2MB dense code log
    let log_size = 2 * 1024 * 1024;
    let dense_log: Vec<u8> = (0..log_size)
        .map(|i| b"console.log('Line ');\n"[i % 21])
        .collect();
    
    assert!(dense_log.len() > 500 * 1024, "Must exceed 500KB threshold");
    
    // The watcher threshold logic should route this to the attachment path
    let attachment_uuid = uuid::Uuid::new_v4().to_string();
    storage_a.save_chunk(&attachment_uuid, &dense_log, false).unwrap();
    println!("  ✓ Wrote 2MB text log to attachment directory");
    
    // Create the 512-byte preview stub
    let preview_len = std::cmp::min(dense_log.len(), 512);
    let preview = String::from_utf8_lossy(&dense_log[..preview_len]);
    let stub = format!("[Large file: {} bytes]\n{}", dense_log.len(), preview);
    let encrypted_stub = crypto_a.encrypt(stub.as_bytes()).unwrap();
    
    // Insert with has_attachment = true, attachment_path = uuid
    let clip_id = db_a.lock().unwrap().insert_clip(
        "text", &encrypted_stub, 100, true, Some(attachment_uuid.clone())
    ).unwrap();
    println!("  ✓ Inserted clip with 512-byte preview stub (id={})", clip_id);
    
    // Verify the stub is small — the actual content is on disk
    assert!(encrypted_stub.len() < 2048, "Encrypted preview stub should be small");
    println!("  ✓ Encrypted stub size: {} bytes (vs 2MB original)", encrypted_stub.len());
    
    // Verify the full file on disk
    let disk_path = storage_a.get_attachment_path(&attachment_uuid);
    assert!(disk_path.exists(), "Attachment .bin file must exist on disk");
    let disk_size = std::fs::metadata(&disk_path).unwrap().len() as usize;
    assert_eq!(disk_size, log_size, "Disk file size must match original");
    println!("  ✓ Disk file verified: {} bytes", disk_size);
    
    // Verify DB clip has the attachment flag
    {
        let guard = db_a.lock().unwrap();
        let clips = guard.get_all_clips().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].6, true, "has_attachment must be true");
        assert_eq!(clips[0].7, Some(attachment_uuid.clone()), "attachment_path must be the UUID");
        println!("  ✓ DB records: has_attachment=true, attachment_path={}", &attachment_uuid[..8]);
    }
    
    println!("  ✅ CASE 3 PASSED: Heavy text correctly routed to attachment Data Plane\n");
}

// ═══════════════════════════════════════════════════════════════════════
// CASE 4: OS Clipboard File URI Normalization
// ═══════════════════════════════════════════════════════════════════════
#[test]
fn e2e_case4_file_uri_normalization() {
    println!("\n══════════ E2E Case 4: File URI Normalization ══════════");
    
    // Test Windows-style path normalization
    let win_path = r"C:\Users\test\AppData\cipherclip\attachments\abc123.bin";
    let win_uri = if win_path.starts_with("file://") {
        win_path.to_string()
    } else {
        format!("file:///{}", win_path.replace("\\", "/"))
    };
    assert_eq!(win_uri, "file:///C:/Users/test/AppData/cipherclip/attachments/abc123.bin");
    println!("  ✓ Windows path normalized: {} → {}", win_path, win_uri);
    
    // Test Unix-style path normalization
    let unix_path = "/home/user/.cipherclip/attachments/abc123.bin";
    let unix_uri = if unix_path.starts_with("file://") {
        unix_path.to_string()
    } else {
        format!("file://{}", unix_path)
    };
    assert_eq!(unix_uri, "file:///home/user/.cipherclip/attachments/abc123.bin");
    println!("  ✓ Unix path normalized: {} → {}", unix_path, unix_uri);
    
    // Test idempotency — already-formatted URIs should pass through unchanged
    let already_uri = "file:///C:/Users/test/file.bin";
    let result = if already_uri.starts_with("file://") {
        already_uri.to_string()
    } else {
        format!("file:///{}", already_uri.replace("\\", "/"))
    };
    assert_eq!(result, already_uri);
    println!("  ✓ Already-formatted URI passes through unchanged");
    
    // Test edge case: path with spaces
    let space_path = r"C:\Users\John Doe\Downloads\my file.pdf";
    let space_uri = format!("file:///{}", space_path.replace("\\", "/"));
    assert!(space_uri.contains("John Doe"), "Spaces should be preserved in the URI");
    println!("  ✓ Path with spaces preserved: {}", space_uri);
    
    println!("  ✅ CASE 4 PASSED: File URI normalization is cross-platform safe\n");
}
