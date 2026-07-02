use rusqlite::{Connection, OptionalExtension, Result as SqlResult};
use sha2::{Sha256, Digest};
use std::path::PathBuf;

pub struct Database {
    conn: Connection,
    pub device_id: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ClipItem {
    pub id: i64,
    pub content_type: String, // "text" or "image"
    pub content: String,
    pub timestamp: i64,
    pub is_locked: bool,
    pub has_attachment: bool,
    pub attachment_path: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct KnownPeer {
    pub device_id: String,
    pub name: String,
    pub last_seen: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SyncEvent {
    pub event_type: String,
    pub clip_uuid: String,
    pub device_id: String,
    pub vector_clock: i64,
    pub timestamp: i64,
    pub content_type: Option<String>,
    pub payload: Option<Vec<u8>>,
    pub has_attachment: Option<bool>,
    pub attachment_path: Option<String>,
    pub pinned: Option<bool>,
    pub is_locked: Option<bool>,
}

impl Database {
    pub fn new(app_dir: PathBuf, device_id: String) -> SqlResult<Self> {
        std::fs::create_dir_all(&app_dir).unwrap_or_default();
        let db_path = app_dir.join("history.db");
        let conn = Connection::open(db_path)?;

        // Create table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content_type TEXT NOT NULL,
                encrypted_payload BLOB NOT NULL,
                timestamp INTEGER NOT NULL,
                pinned BOOLEAN NOT NULL DEFAULT 0,
                is_deleted BOOLEAN NOT NULL DEFAULT 0,
                is_locked BOOLEAN NOT NULL DEFAULT 0,
                payload_hash TEXT,
                has_attachment BOOLEAN NOT NULL DEFAULT 0,
                attachment_path TEXT,
                uuid TEXT UNIQUE
            )",
            (),
        )?;

        
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN pinned BOOLEAN NOT NULL DEFAULT 0", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN is_deleted BOOLEAN NOT NULL DEFAULT 0", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN is_locked BOOLEAN NOT NULL DEFAULT 0", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN payload_hash TEXT", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN has_attachment BOOLEAN NOT NULL DEFAULT 0", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN attachment_path TEXT", ());
        let _ = conn.execute("ALTER TABLE clipboard_history ADD COLUMN uuid TEXT", ());
        let _ = conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_clipboard_history_uuid ON clipboard_history(uuid)", ());

        // Event-sourcing CRDT tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                clip_uuid TEXT NOT NULL,
                device_id TEXT NOT NULL,
                vector_clock INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                content_type TEXT,
                payload BLOB,
                has_attachment BOOLEAN DEFAULT 0,
                attachment_path TEXT
            )",
            (),
        )?;

        // Ensure event_log has the newer columns for users upgrading from older versions
        let _ = conn.execute("ALTER TABLE event_log ADD COLUMN has_attachment BOOLEAN DEFAULT 0", ());
        let _ = conn.execute("ALTER TABLE event_log ADD COLUMN attachment_path TEXT", ());
        let _ = conn.execute("ALTER TABLE event_log ADD COLUMN content_type TEXT", ());
        let _ = conn.execute("ALTER TABLE event_log ADD COLUMN payload BLOB", ());

        // Event-sourcing CRDT tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                clip_uuid TEXT NOT NULL,
                device_id TEXT NOT NULL,
                vector_clock INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                content_type TEXT,
                payload BLOB,
                has_attachment BOOLEAN DEFAULT 0,
                attachment_path TEXT
            )",
            (),
        )?;

        // Deduplicate any existing duplicate rows (from the runaway catch-up bug).
        // Keep the row with the lowest rowid for each (clip_uuid, device_id, vector_clock)
        let _ = conn.execute(
            "DELETE FROM event_log WHERE id NOT IN (
                SELECT MIN(id) FROM event_log
                GROUP BY clip_uuid, device_id, vector_clock
            )",
            (),
        );

        // Now add the unique index so it can never happen again.
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_event_log_unique 
             ON event_log(clip_uuid, device_id, vector_clock)",
            (),
        );

        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_event_log_vector_clock ON event_log(vector_clock)",
            (),
        );

        conn.execute(
            "CREATE TABLE IF NOT EXISTS peer_sync_state (
                author_id TEXT NOT NULL,
                peer_id TEXT NOT NULL,
                acknowledged_clock INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (author_id, peer_id)
            )",
            (),
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS known_peers (
                device_id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                last_seen INTEGER NOT NULL DEFAULT 0
            )",
            (),
        )?;

        Ok(Self { conn, device_id })
    }

    pub fn insert_clip(
        &self,
        content_type: &str,
        encrypted_payload: &[u8],
        limit: i64,
        has_attachment: bool,
        attachment_path: Option<String>,
    ) -> SqlResult<i64> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        let mut hasher = Sha256::new();
        hasher.update(encrypted_payload);
        let hash = hex::encode(hasher.finalize());

        let uuid = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO clipboard_history (uuid, content_type, encrypted_payload, timestamp, pinned, is_deleted, is_locked, payload_hash, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, 0, 0, 0, ?5, ?6, ?7)",
            rusqlite::params![&uuid, content_type, encrypted_payload, timestamp, hash, has_attachment, attachment_path.clone()],
        )?;
        
        let event_type = "INSERT";
        let vector_clock = self.get_next_hlc();
        self.conn.execute(
            "INSERT INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, content_type, payload, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![event_type, &uuid, &self.device_id, vector_clock, timestamp, content_type, encrypted_payload, has_attachment, attachment_path],
        )?;

        let new_id = self.conn.last_insert_rowid();

        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clipboard_history WHERE pinned = 0 AND is_locked = 0 AND is_deleted = 0",
            [],
            |row| row.get(0),
        )?;

        let actual_limit = if limit <= 0 { 100 } else { limit };
        if count > actual_limit {
            let to_delete = count - actual_limit;
            if to_delete > 0 {
                self.conn.execute(
                    "UPDATE clipboard_history SET is_deleted = 1, encrypted_payload = NULL WHERE id IN (SELECT id FROM clipboard_history WHERE pinned = 0 AND is_locked = 0 AND is_deleted = 0 ORDER BY timestamp ASC, id ASC LIMIT ?1)",
                    rusqlite::params![to_delete],
                )?;
            }
        }

        let seven_days_ago = timestamp - (7 * 24 * 60 * 60);
        let _ = self.conn.execute(
            "DELETE FROM clipboard_history WHERE is_deleted = 1 AND timestamp < ?1",
            (seven_days_ago,),
        );

        Ok(new_id)
    }

    
    pub fn get_uuid_by_id(&self, id: i64) -> SqlResult<Option<String>> {
        self.conn.query_row("SELECT uuid FROM clipboard_history WHERE id = ?1", [id], |row| row.get(0)).optional()
    }

    pub fn toggle_pin(&self, id: i64, pinned: bool) -> SqlResult<()> {

        let (uuid, has_attachment, attachment_path): (String, bool, Option<String>) = self.conn.query_row(
            "SELECT uuid, has_attachment, attachment_path FROM clipboard_history WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        )?;

        self.conn.execute(
            "UPDATE clipboard_history SET pinned = ?1 WHERE id = ?2",
            rusqlite::params![pinned, id],
        )?;
        
        let vector_clock = self.get_next_hlc();
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
        let event_type = if pinned { "PIN" } else { "UNPIN" };
        self.conn.execute(
            "INSERT INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![event_type, &uuid, &self.device_id, vector_clock, timestamp, has_attachment, attachment_path],
        )?;
        Ok(())
    }

    pub fn toggle_lock(&self, id: i64, is_locked: bool) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_locked = ?1 WHERE id = ?2",
            (is_locked, id),
        )?;
        // ONLY sync the LOCK action. Unlock is always local-only.
        if is_locked {
            if let Some(uuid) = self.get_uuid_by_id(id)? {
                let vector_clock = self.get_next_hlc();
                let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
                let _ = self.conn.execute(
                    "INSERT OR IGNORE INTO event_log \
                     (event_type, clip_uuid, device_id, vector_clock, timestamp, has_attachment, attachment_path) \
                     VALUES ('LOCK', ?1, ?2, ?3, ?4, 0, NULL)",
                    rusqlite::params![&uuid, &self.device_id, vector_clock, timestamp],
                );
            }
        }
        Ok(())
    }

    pub fn toggle_lock_by_hash(&self, hash: &str, is_locked: bool) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_locked = ?1 WHERE payload_hash = ?2",
            (is_locked, hash),
        )?;
        // Not pushing event for hash directly unless we find uuid
        Ok(())
    }

    pub fn is_locked_by_hash(&self, hash: &str) -> SqlResult<bool> {
        let is_locked: bool = self.conn.query_row(
            "SELECT is_locked FROM clipboard_history WHERE payload_hash = ?1",
            (hash,),
            |row| row.get(0),
        ).unwrap_or(false);
        Ok(is_locked)
    }

    pub fn toggle_pin_by_hash(&self, hash: &str, pinned: bool) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET pinned = ?1 WHERE payload_hash = ?2",
            (pinned, hash),
        )?;
        Ok(())
    }

    pub fn delete_clip_by_hash(&self, hash: &str) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_deleted = 1 WHERE payload_hash = ?1",
            (hash,),
        )?;
        Ok(())
    }

    pub fn get_hash_by_id(&self, id: i64) -> SqlResult<Option<String>> {
        let hash: Option<String> = self.conn.query_row(
            "SELECT payload_hash FROM clipboard_history WHERE id = ?1",
            (id,),
            |row| row.get(0),
        )?;
        Ok(hash)
    }

    pub fn delete_clip(&self, id: i64) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_deleted = 1, pinned = 0 WHERE id = ?1",
            (id,),
        )?;
        // Record a DELETE event so the deletion propagates during sync
        if let Some(uuid) = self.get_uuid_by_id(id)? {
            let vector_clock = self.get_next_hlc();
            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
            let _ = self.conn.execute(
                "INSERT INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["DELETE", &uuid, &self.device_id, vector_clock, timestamp, false, None::<String>],
            );
        }
        Ok(())
    }

    pub fn permanently_delete_clip(&self, id: i64) -> SqlResult<()> {
        if let Some(uuid) = self.get_uuid_by_id(id)? {
            let vector_clock = self.get_next_hlc();
            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
            let _ = self.conn.execute(
                "INSERT INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["DELETE", &uuid, &self.device_id, vector_clock, timestamp, false, None::<String>],
            );
        }
        self.conn
            .execute("DELETE FROM clipboard_history WHERE id = ?1", (id,))?;
        Ok(())
    }

    pub fn clear_all_locks(&self) -> SqlResult<()> {
        self.conn
            .execute("UPDATE clipboard_history SET is_locked = 0", [])?;
        Ok(())
    }

    pub fn restore_clip(&self, id: i64) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_deleted = 0 WHERE id = ?1",
            (id,),
        )?;
        Ok(())
    }

    pub fn get_latest_hash(&self) -> SqlResult<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT encrypted_payload FROM clipboard_history WHERE is_deleted = 0 ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn get_all_clips(&self) -> SqlResult<Vec<(i64, String, Vec<u8>, i64, bool, bool, bool, Option<String>)>> {
        // Return 100 for rendering regardless of limit
        let mut stmt = self.conn.prepare("SELECT id, content_type, encrypted_payload, timestamp, pinned, is_locked, has_attachment, attachment_path FROM clipboard_history WHERE is_deleted = 0 ORDER BY id DESC LIMIT 100")?;
        let clip_iter = stmt.query_map([], |row| {
            let encrypted_payload: Option<Vec<u8>> = row.get(2).ok();
            let attachment_path: Option<String> = row.get(7).unwrap_or(None);
            Ok((
                row.get(0).unwrap_or(0),
                row.get(1).unwrap_or_else(|_| "text".to_string()),
                encrypted_payload.unwrap_or_default(),
                row.get(3).unwrap_or(0),
                row.get(4).unwrap_or(false),
                row.get(5).unwrap_or(false),
                row.get(6).unwrap_or(false),
                attachment_path,
            ))
        })?;

        let mut clips = Vec::new();
        for clip in clip_iter {
            clips.push(clip?);
        }
        Ok(clips)
    }

    pub fn get_clip_by_id(&self, id: i64) -> SqlResult<Option<(String, Vec<u8>)>> {
        self.conn
            .query_row(
                "SELECT content_type, encrypted_payload FROM clipboard_history WHERE id = ?1",
                (id,),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    pub fn get_deleted_clips(&self) -> SqlResult<Vec<(i64, String, Vec<u8>, i64, bool, bool, bool, Option<String>)>> {
        let mut stmt = self.conn.prepare("SELECT id, content_type, encrypted_payload, timestamp, pinned, is_locked, has_attachment, attachment_path FROM clipboard_history WHERE is_deleted = 1 ORDER BY id DESC")?;
        let clip_iter = stmt.query_map([], |row| {
            let encrypted_payload: Option<Vec<u8>> = row.get(2).ok();
            let attachment_path: Option<String> = row.get(7).unwrap_or(None);
            Ok((
                row.get(0).unwrap_or(0),
                row.get(1).unwrap_or_else(|_| "text".to_string()),
                encrypted_payload.unwrap_or_default(),
                row.get(3).unwrap_or(0),
                row.get(4).unwrap_or(false),
                row.get(5).unwrap_or(false),
                row.get(6).unwrap_or(false),
                attachment_path,
            ))
        })?;

        let mut clips = Vec::new();
        for clip in clip_iter {
            clips.push(clip?);
        }
        Ok(clips)
    }

    pub fn empty_recycle_bin(&self) -> SqlResult<()> {
        self.conn
            .execute("DELETE FROM clipboard_history WHERE is_deleted = 1", ())?;
        Ok(())
    }

    pub fn clear_all(&self, delete_locked: bool) -> SqlResult<()> {
        let mut uuids = Vec::new();
        if delete_locked {
            let mut stmt = self.conn.prepare("SELECT uuid FROM clipboard_history")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                uuids.push(row.get::<_, String>(0)?);
            }
            self.conn.execute("UPDATE clipboard_history SET is_deleted = 1", ())?;
        } else {
            let mut stmt = self.conn.prepare("SELECT uuid FROM clipboard_history WHERE is_locked = 0")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                uuids.push(row.get::<_, String>(0)?);
            }
            self.conn.execute("UPDATE clipboard_history SET is_deleted = 1 WHERE is_locked = 0", ())?;
        }
        
        let vector_clock = self.get_next_hlc();
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
        for uuid in uuids {
            let _ = self.conn.execute(
                "INSERT INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["DELETE", &uuid, &self.device_id, vector_clock, timestamp, false, None::<String>],
            );
        }
        Ok(())
    }

    pub fn get_next_hlc(&self) -> i64 {
        let physical = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
        let max_db_clock: i64 = self.conn.query_row(
            "SELECT MAX(vector_clock) FROM event_log",
            [],
            |row| row.get(0),
        ).unwrap_or(0);
        std::cmp::max(physical, max_db_clock + 1)
    }

    pub fn get_latest_event_uuid(&self) -> SqlResult<Option<String>> {
        self.conn.query_row(
            "SELECT clip_uuid FROM event_log ORDER BY vector_clock DESC LIMIT 1",
            [],
            |row| row.get(0),
        ).optional()
    }

    pub fn get_latest_hlc(&self) -> i64 {
        self.conn.query_row(
            "SELECT COALESCE(MAX(vector_clock), 0) FROM event_log WHERE device_id = ?1",
            [&self.device_id],
            |row| row.get(0),
        ).unwrap_or(0)
    }

    pub fn get_events_since_hlc(&self, hlc_cursor: i64) -> SqlResult<Vec<SyncEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.event_type, e.clip_uuid, e.device_id, e.vector_clock, e.timestamp, 
                    COALESCE(e.content_type, c.content_type) as content_type, 
                    COALESCE(e.payload, c.encrypted_payload) as payload, 
                    COALESCE(e.has_attachment, c.has_attachment, 0) as has_attachment, 
                    COALESCE(e.attachment_path, c.attachment_path) as attachment_path, 
                    c.pinned, c.is_locked, c.is_deleted 
             FROM event_log e 
             LEFT JOIN clipboard_history c ON e.clip_uuid = c.uuid 
             WHERE e.vector_clock > ?1
             ORDER BY e.vector_clock ASC"
        )?;

        let event_iter = stmt.query_map([hlc_cursor], |row| {
            let mut event_type: String = row.get(0)?;
            let is_deleted: Option<bool> = row.get(11).unwrap_or(None);
            let mut payload: Option<Vec<u8>> = row.get(6).ok();
            
            if is_deleted == Some(true) && event_type != "DELETE" {
                event_type = "DELETE".to_string();
                payload = None;
            }

            Ok(SyncEvent {
                event_type,
                clip_uuid: row.get(1)?,
                device_id: row.get(2)?,
                vector_clock: row.get(3)?,
                timestamp: row.get(4)?,
                content_type: row.get(5).ok(),
                payload,
                has_attachment: row.get(7).ok(),
                attachment_path: row.get(8).ok(),
                pinned: row.get(9).ok(),
                is_locked: row.get(10).ok(),
            })
        })?;

        let mut events = Vec::new();
        for event in event_iter {
            if let Ok(evt) = event { events.push(evt); }
        }
        Ok(events)
    }

    pub fn upsert_known_peer(&self, device_id: &str, name: &str) -> SqlResult<()> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        self.conn.execute(
            "INSERT INTO known_peers (device_id, name, last_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id) DO UPDATE SET name = excluded.name, last_seen = excluded.last_seen",
            rusqlite::params![device_id, name, timestamp],
        )?;
        Ok(())
    }

    pub fn get_known_peers(&self) -> SqlResult<Vec<KnownPeer>> {
        let mut stmt = self.conn.prepare("SELECT device_id, name, last_seen FROM known_peers ORDER BY last_seen DESC")?;
        let peers = stmt.query_map([], |row| {
            Ok(KnownPeer {
                device_id: row.get(0)?,
                name: row.get(1)?,
                last_seen: row.get(2)?,
            })
        })?;
        let mut result = Vec::new();
        for p in peers {
            if let Ok(peer) = p {
                result.push(peer);
            }
        }
        Ok(result)
    }

    pub fn get_all_sync_states(&self) -> SqlResult<std::collections::HashMap<String, i64>> {
        let mut stmt = self.conn.prepare("SELECT author_id, acknowledged_clock FROM peer_sync_state WHERE peer_id = ?1")?;
        let states = stmt.query_map([&self.device_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        let mut map = std::collections::HashMap::new();
        for s in states {
            if let Ok((peer_id, clock)) = s {
                map.insert(peer_id, clock);
            }
        }
        Ok(map)
    }

    pub fn get_peer_sync_state(&self, author_id: &str) -> SqlResult<i64> {
        let clock: i64 = self.conn.query_row(
            "SELECT acknowledged_clock FROM peer_sync_state WHERE peer_id = ?1 AND author_id = ?2",
            [&self.device_id, author_id],
            |row| row.get(0),
        ).unwrap_or(0);
        Ok(clock)
    }
    
    pub fn update_peer_sync_state(&self, peer_id: &str, clock: i64) -> SqlResult<()> {
        self.conn.execute(
            "INSERT INTO peer_sync_state (author_id, peer_id, acknowledged_clock) VALUES (?1, ?2, ?3)
             ON CONFLICT(author_id, peer_id) DO UPDATE SET acknowledged_clock = excluded.acknowledged_clock",
            rusqlite::params![&self.device_id, peer_id, clock],
        )?;
        Ok(())
    }

    pub fn remove_peer_sync_state(&self, peer_id: &str) -> SqlResult<()> {
        self.conn.execute("DELETE FROM peer_sync_state WHERE peer_id = ?1", [peer_id])?;
        self.conn.execute("DELETE FROM known_peers WHERE device_id = ?1", [peer_id])?;
        Ok(())
    }

    pub fn clear_all_peers(&self) -> SqlResult<()> {
        self.conn.execute("DELETE FROM peer_sync_state", [])?;
        self.conn.execute("DELETE FROM known_peers", [])?;
        Ok(())
    }

    pub fn apply_sync_events(&self, events: Vec<SyncEvent>) -> SqlResult<()> {
        let current_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as i64;
        for evt in events {
            if evt.timestamp > current_time + 3600_000_000 { continue; }

            let min_clock: i64 = self.conn.query_row(
                "SELECT acknowledged_clock FROM peer_sync_state WHERE author_id = ?1 AND peer_id = ?2",
                [&evt.device_id, &self.device_id],
                |r| r.get(0)
            ).unwrap_or(0);
            
            if evt.vector_clock <= min_clock {
                continue;
            }
            
            let existing_latest: Option<(i64, String)> = self.conn.query_row(
                "SELECT timestamp, device_id FROM event_log WHERE clip_uuid = ?1 ORDER BY timestamp DESC, device_id DESC LIMIT 1",
                [&evt.clip_uuid],
                |row| Ok((row.get(0)?, row.get(1)?))
            ).optional()?;

            let mut should_apply = true;
            if let Some((ext_time, ext_dev)) = existing_latest {
                if ext_time > evt.timestamp || (ext_time == evt.timestamp && ext_dev >= evt.device_id) {
                    should_apply = false;
                }
            }

            if should_apply {
                if evt.event_type == "INSERT" || evt.event_type == "UPDATE" {
                    let has_attach = evt.has_attachment.unwrap_or(false);
                    let attach_path = evt.attachment_path.clone();
                    let is_pinned = evt.pinned.unwrap_or(false);
                    let is_locked = evt.is_locked.unwrap_or(false);

                    if let Some(ctype) = &evt.content_type {
                        if let Some(payload) = &evt.payload {
                            // Normal clip with inline payload
                            self.conn.execute(
                                "INSERT INTO clipboard_history (uuid, content_type, encrypted_payload, timestamp, pinned, is_locked, is_deleted, has_attachment, attachment_path) 
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)
                                 ON CONFLICT(uuid) DO UPDATE SET 
                                 content_type=excluded.content_type, 
                                 encrypted_payload=excluded.encrypted_payload, 
                                 timestamp=excluded.timestamp,
                                 pinned=excluded.pinned,
                                 is_locked=excluded.is_locked,
                                 has_attachment=excluded.has_attachment,
                                 attachment_path=excluded.attachment_path",
                                rusqlite::params![&evt.clip_uuid, ctype, payload, evt.timestamp, is_pinned, is_locked, has_attach, attach_path],
                            )?;
                        } else if has_attach {
                            // Attachment-only clip: insert placeholder row so the clip appears in history.
                            // encrypted_payload will be filled in after the BIN_REQ download completes.
                            self.conn.execute(
                                "INSERT INTO clipboard_history (uuid, content_type, encrypted_payload, timestamp, pinned, is_locked, is_deleted, has_attachment, attachment_path) 
                                 VALUES (?1, ?2, x'', ?3, ?4, ?5, 0, ?6, ?7)
                                 ON CONFLICT(uuid) DO UPDATE SET 
                                 content_type=excluded.content_type, 
                                 timestamp=excluded.timestamp,
                                 pinned=excluded.pinned,
                                 is_locked=excluded.is_locked,
                                 has_attachment=excluded.has_attachment,
                                 attachment_path=excluded.attachment_path",
                                rusqlite::params![&evt.clip_uuid, ctype, evt.timestamp, is_pinned, is_locked, has_attach, attach_path],
                            )?;
                        }
                    }
                } else if evt.event_type == "DELETE" {
                      let _ = self.conn.execute(
                          "INSERT INTO clipboard_history (uuid, content_type, encrypted_payload, timestamp, pinned, is_locked, is_deleted, has_attachment, attachment_path) 
                           VALUES (?1, 'text', x'', ?2, 0, 0, 1, 0, NULL)
                           ON CONFLICT(uuid) DO UPDATE SET is_deleted = 1",
                          rusqlite::params![&evt.clip_uuid, evt.timestamp],
                      );
                } else if evt.event_type == "LOCK" {
                    let _ = self.conn.execute(
                        "UPDATE clipboard_history SET is_locked = 1 WHERE uuid = ?1",
                        [&evt.clip_uuid],
                    );
                } else if evt.event_type == "PIN" {
                    let _ = self.conn.execute(
                        "UPDATE clipboard_history SET pinned = 1 WHERE uuid = ?1",
                        [&evt.clip_uuid],
                    );
                } else if evt.event_type == "UNPIN" {
                    let _ = self.conn.execute(
                        "UPDATE clipboard_history SET pinned = 0 WHERE uuid = ?1",
                        [&evt.clip_uuid],
                    );
                }
            }

            let has_attach = evt.has_attachment.unwrap_or(false);
            let attach_path = evt.attachment_path.clone();
            let ctype = evt.content_type.clone();
            let payload_bytes = evt.payload.clone();
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO event_log (event_type, clip_uuid, device_id, vector_clock, timestamp, content_type, payload, has_attachment, attachment_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![&evt.event_type, &evt.clip_uuid, &evt.device_id, evt.vector_clock, evt.timestamp, ctype, payload_bytes, has_attach, attach_path],
            );
            
            let _ = self.conn.execute(
                "INSERT INTO peer_sync_state (author_id, peer_id, acknowledged_clock) VALUES (?1, ?2, ?3)
                 ON CONFLICT(author_id, peer_id) DO UPDATE SET acknowledged_clock = MAX(acknowledged_clock, excluded.acknowledged_clock)",
                rusqlite::params![&evt.device_id, &self.device_id, evt.vector_clock],
            );
        }
        Ok(())
    }

    pub fn get_missing_events(&self, peer_clocks: &std::collections::HashMap<String, i64>, limit: i64) -> SqlResult<Vec<SyncEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.event_type, e.clip_uuid, e.device_id, e.vector_clock, e.timestamp, 
                    COALESCE(e.content_type, c.content_type) as content_type, 
                    COALESCE(e.payload, c.encrypted_payload) as payload, 
                    COALESCE(e.has_attachment, c.has_attachment, 0) as has_attachment, 
                    COALESCE(e.attachment_path, c.attachment_path) as attachment_path, 
                    c.pinned, c.is_locked, c.is_deleted 
             FROM event_log e 
             LEFT JOIN clipboard_history c ON e.clip_uuid = c.uuid 
             ORDER BY e.id ASC"
        )?;
        
        let event_iter = stmt.query_map([], |row| {
            let mut event_type: String = row.get(0)?;
            let is_deleted: Option<bool> = row.get(11).unwrap_or(None);
            let mut payload: Option<Vec<u8>> = row.get(6).ok();
            
            if is_deleted == Some(true) && event_type != "DELETE" {
                event_type = "DELETE".to_string();
                payload = None;
            }

            Ok(SyncEvent {
                event_type,
                clip_uuid: row.get(1)?,
                device_id: row.get(2)?,
                vector_clock: row.get(3)?,
                timestamp: row.get(4)?,
                content_type: row.get(5).ok(),
                payload,
                has_attachment: row.get(7).ok(),
                attachment_path: row.get(8).ok(),
                pinned: row.get(9).ok(),
                is_locked: row.get(10).ok(),
            })
        })?;

        let mut missing_events = Vec::new();
        for event in event_iter {
            if let Ok(evt) = event {
                let peer_has = peer_clocks.get(&evt.device_id).copied().unwrap_or(0);
                if evt.vector_clock > peer_has {
                    missing_events.push(evt);
                    if missing_events.len() as i64 >= limit { break; }
                }
            }
        }
        Ok(missing_events)
    }

    pub fn prune_tombstones(&self) -> SqlResult<()> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM peer_sync_state", [], |r| r.get(0)).unwrap_or(0);
        if count == 0 { return Ok(()); }
        let min_clock: i64 = self.conn.query_row("SELECT MIN(acknowledged_clock) FROM peer_sync_state WHERE author_id = ?1", [&self.device_id], |r| r.get(0)).unwrap_or(0);
        self.conn.execute("DELETE FROM event_log WHERE vector_clock <= ?1 AND device_id = ?2", rusqlite::params![min_clock, &self.device_id])?;
        Ok(())
    }

    pub fn get_recent_events(&self, limit: i64) -> SqlResult<Vec<SyncEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.event_type, e.clip_uuid, e.device_id, e.vector_clock, e.timestamp, 
                    COALESCE(e.content_type, c.content_type) as content_type, 
                    COALESCE(e.payload, c.encrypted_payload) as payload, 
                    COALESCE(e.has_attachment, c.has_attachment, 0) as has_attachment, 
                    COALESCE(e.attachment_path, c.attachment_path) as attachment_path, 
                    c.pinned, c.is_locked, c.is_deleted 
             FROM event_log e 
             LEFT JOIN clipboard_history c ON e.clip_uuid = c.uuid 
             WHERE e.device_id = ?1
             ORDER BY e.id DESC LIMIT ?2"
        )?;

        let event_iter = stmt.query_map(rusqlite::params![&self.device_id, limit], |row| {
            let mut event_type: String = row.get(0)?;
            let is_deleted: Option<bool> = row.get(11).unwrap_or(None);
            let mut payload: Option<Vec<u8>> = row.get(6).ok();
            
            if is_deleted == Some(true) && event_type != "DELETE" {
                event_type = "DELETE".to_string();
                payload = None;
            }

            Ok(SyncEvent {
                event_type,
                clip_uuid: row.get(1)?,
                device_id: row.get(2)?,
                vector_clock: row.get(3)?,
                timestamp: row.get(4)?,
                content_type: row.get(5).ok(),
                payload,
                has_attachment: row.get(7).ok(),
                attachment_path: row.get(8).ok(),
                pinned: row.get(9).ok(),
                is_locked: row.get(10).ok(),
            })
        })?;

        let mut events = Vec::new();
        for event in event_iter {
            if let Ok(evt) = event { events.push(evt); }
        }
        events.reverse();
        Ok(events)
    }

}
