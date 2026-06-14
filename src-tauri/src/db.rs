use rusqlite::{Connection, OptionalExtension, Result as SqlResult};
use std::path::PathBuf;

pub struct Database {
    conn: Connection,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ClipItem {
    pub id: i64,
    pub content_type: String, // "text" or "image"
    pub content: String, // For text: actual text. For image: base64 maybe? Or handle separately. Let's start with returning string.
    pub timestamp: i64,
    pub is_locked: bool,
}

impl Database {
    pub fn new(app_dir: PathBuf) -> SqlResult<Self> {
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
                is_locked BOOLEAN NOT NULL DEFAULT 0
            )",
            (),
        )?;

        // Simple migrations
        let _ = conn.execute(
            "ALTER TABLE clipboard_history ADD COLUMN pinned BOOLEAN NOT NULL DEFAULT 0",
            (),
        );
        let _ = conn.execute(
            "ALTER TABLE clipboard_history ADD COLUMN is_deleted BOOLEAN NOT NULL DEFAULT 0",
            (),
        );
        let _ = conn.execute(
            "ALTER TABLE clipboard_history ADD COLUMN is_locked BOOLEAN NOT NULL DEFAULT 0",
            (),
        );

        Ok(Self { conn })
    }

    pub fn insert_clip(
        &self,
        content_type: &str,
        encrypted_payload: &[u8],
        limit: i64,
    ) -> SqlResult<i64> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT INTO clipboard_history (content_type, encrypted_payload, timestamp, pinned, is_deleted, is_locked) VALUES (?1, ?2, ?3, 0, 0, 0)",
            (content_type, encrypted_payload, timestamp),
        )?;

        let new_id = self.conn.last_insert_rowid();

        // Auto-delete oldest unpinned clip if over limit
        // We find how many unpinned items there are.
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clipboard_history WHERE pinned = 0 AND is_locked = 0",
            [],
            |row| row.get(0),
        )?;

        if count > limit {
            // Delete the oldest unpinned items
            let to_delete = count - limit;
            self.conn.execute(
                "DELETE FROM clipboard_history WHERE id IN (
                    SELECT id FROM clipboard_history WHERE pinned = 0 AND is_locked = 0 ORDER BY timestamp ASC LIMIT ?1
                )",
                (to_delete,),
            )?;
        }

        // Cleanup old recycled items (> 7 days)
        let seven_days_ago = timestamp - (7 * 24 * 60 * 60);
        let _ = self.conn.execute(
            "DELETE FROM clipboard_history WHERE is_deleted = 1 AND timestamp < ?1",
            (seven_days_ago,),
        );

        Ok(new_id)
    }

    pub fn toggle_pin(&self, id: i64, pinned: bool) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET pinned = ?1 WHERE id = ?2",
            (pinned, id),
        )?;
        Ok(())
    }

    pub fn toggle_lock(&self, id: i64, is_locked: bool) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_locked = ?1 WHERE id = ?2",
            (is_locked, id),
        )?;
        Ok(())
    }

    pub fn delete_clip(&self, id: i64) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE clipboard_history SET is_deleted = 1, pinned = 0 WHERE id = ?1",
            (id,),
        )?;
        Ok(())
    }

    pub fn permanently_delete_clip(&self, id: i64) -> SqlResult<()> {
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
                "SELECT encrypted_payload FROM clipboard_history ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn get_all_clips(&self) -> SqlResult<Vec<(i64, String, Vec<u8>, i64, bool, bool)>> {
        // Return 100 for rendering regardless of limit
        let mut stmt = self.conn.prepare("SELECT id, content_type, encrypted_payload, timestamp, pinned, is_locked FROM clipboard_history WHERE is_deleted = 0 ORDER BY id DESC LIMIT 100")?;
        let clip_iter = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
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

    pub fn get_deleted_clips(&self) -> SqlResult<Vec<(i64, String, Vec<u8>, i64, bool, bool)>> {
        let mut stmt = self.conn.prepare("SELECT id, content_type, encrypted_payload, timestamp, pinned, is_locked FROM clipboard_history WHERE is_deleted = 1 ORDER BY id DESC LIMIT 100")?;
        let clip_iter = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
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
        if delete_locked {
            self.conn.execute("DELETE FROM clipboard_history", ())?;
        } else {
            self.conn
                .execute("DELETE FROM clipboard_history WHERE is_locked = 0", ())?;
        }
        Ok(())
    }
}
