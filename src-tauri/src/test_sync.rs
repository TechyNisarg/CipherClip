use app_lib::db::Database;
use app_lib::db::SyncEvent;
use std::sync::{Arc, Mutex};
fn main() {
    let db = Database::new("test_sync.db".into(), "device1".into()).unwrap();
    let evt = SyncEvent {
        event_type: "INSERT".to_string(),
        clip_uuid: "1234".to_string(),
        device_id: "device1".to_string(),
        vector_clock: 1,
        timestamp: 1234567890,
        content_type: Some("text".to_string()),
        payload: Some(vec![1, 2, 3]),
        has_attachment: Some(false),
        attachment_path: None,
        pinned: Some(false),
        is_locked: Some(false),
    };
    match db.apply_sync_events(vec![evt]) {
        Ok(_) => println!("Success!"),
        Err(e) => println!("Error: {:?}", e),
    }
}
