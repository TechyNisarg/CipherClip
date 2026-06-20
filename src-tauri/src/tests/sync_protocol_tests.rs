#[cfg(test)]
mod tests {
    use crate::db::Database;
    
    use std::collections::HashMap;

    fn setup_test_db(name: &str) -> (Database, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Database::new(temp_dir.path().to_path_buf(), name.to_string()).unwrap();
        (db, temp_dir)
    }

    #[test]
    fn test_concurrent_offline_resolution() {
        let (db_a, _dir_a) = setup_test_db("Device_A");
        let (db_b, _dir_b) = setup_test_db("Device_B");

        let clip_id_a = db_a.insert_clip("TEXT", b"Hello", 100, false, None).unwrap();

        let events_from_a = db_a.get_missing_events(&HashMap::new(), 50).unwrap();
        db_b.apply_sync_events(events_from_a.clone()).unwrap();
        let max_clock_a = events_from_a.iter().map(|e| e.vector_clock).max().unwrap_or(0);
        db_b.update_peer_sync_state("Device_A", max_clock_a).unwrap();

        let clips_b = db_b.get_all_clips().unwrap();
        assert_eq!(clips_b.len(), 1);
        let clip_id_b = clips_b[0].0;

        std::thread::sleep(std::time::Duration::from_millis(5));
        db_a.toggle_pin(clip_id_a, true).unwrap();
        
        std::thread::sleep(std::time::Duration::from_millis(5));
        db_b.delete_clip(clip_id_b).unwrap();

        let mut map_a = HashMap::new();
        map_a.insert("Device_A".to_string(), db_b.get_peer_sync_state("Device_A").unwrap_or(0));
        let missing_for_b = db_a.get_missing_events(&map_a, 50).unwrap();

        let mut map_b = HashMap::new();
        map_b.insert("Device_B".to_string(), db_a.get_peer_sync_state("Device_B").unwrap_or(0));
        let missing_for_a = db_b.get_missing_events(&map_b, 50).unwrap();

        db_b.apply_sync_events(missing_for_b).unwrap();
        db_a.apply_sync_events(missing_for_a).unwrap();

        let a_clips = db_a.get_all_clips().unwrap();
        let b_clips = db_b.get_all_clips().unwrap();

        assert_eq!(a_clips.len(), 0);
        assert_eq!(b_clips.len(), 0);
    }

    #[test]
    fn test_offline_reconnection_catchup() {
        let (db_a, _dir_a) = setup_test_db("Device_A");
        let (db_b, _dir_b) = setup_test_db("Device_B");

        for i in 0..10 {
            db_a.insert_clip("TEXT", format!("Clip {}", i).as_bytes(), 100, false, None).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let missing_events = db_a.get_missing_events(&HashMap::new(), 50).unwrap();
        assert_eq!(missing_events.len(), 10);

        db_b.apply_sync_events(missing_events).unwrap();

        let b_clips = db_b.get_all_clips().unwrap();
        assert_eq!(b_clips.len(), 10);
    }

    #[test]
    fn test_tombstone_sync_and_compaction() {
        let (db_a, _dir_a) = setup_test_db("Device_A");
        let (db_b, _dir_b) = setup_test_db("Device_B");

        let id_a = db_a.insert_clip("TEXT", b"To Be Deleted", 100, false, None).unwrap();
        let evts = db_a.get_missing_events(&HashMap::new(), 50).unwrap();
        db_b.apply_sync_events(evts.clone()).unwrap();
        let max_clock_a2 = evts.iter().map(|e| e.vector_clock).max().unwrap_or(0);
        db_b.update_peer_sync_state("Device_A", max_clock_a2).unwrap();

        assert_eq!(db_b.get_all_clips().unwrap().len(), 1);

        std::thread::sleep(std::time::Duration::from_millis(5));
        db_a.delete_clip(id_a).unwrap();
        println!("A clips after delete: {}", db_a.get_all_clips().unwrap().len());

        let mut map_b = HashMap::new();
        map_b.insert("Device_A".to_string(), db_b.get_peer_sync_state("Device_A").unwrap_or(0));
        let evts2 = db_a.get_missing_events(&map_b, 50).unwrap();

        assert_eq!(evts2.len(), 1);
        assert_eq!(evts2[0].event_type, "DELETE");

        db_b.apply_sync_events(evts2).unwrap();

        let b_clips = db_b.get_all_clips().unwrap();
        println!("B clips at end: {}", b_clips.len());
        for clip in b_clips {
            let hash = db_b.get_hash_by_id(clip.0).unwrap();
            println!("B clip id: {}, uuid: {:?}", clip.0, hash);
        }
        assert_eq!(db_b.get_all_clips().unwrap().len(), 0);
        assert_eq!(db_a.get_all_clips().unwrap().len(), 0);
    }
}
