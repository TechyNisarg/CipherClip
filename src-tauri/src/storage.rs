use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

pub struct StorageManager {
    pub attachments_dir: PathBuf,
}

impl StorageManager {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        let app_dir = app_data_dir;
        
        let attachments_dir = app_dir.join("attachments");
        if !attachments_dir.exists() {
            fs::create_dir_all(&attachments_dir).map_err(|e| e.to_string())?;
        }
        
        Ok(Self { attachments_dir })
    }

    pub fn get_attachment_path(&self, uuid: &str) -> PathBuf {
        self.attachments_dir.join(format!("{}.png", uuid))
    }

    pub fn get_legacy_attachment_path(&self, uuid: &str) -> PathBuf {
        self.attachments_dir.join(format!("{}.bin", uuid))
    }

    pub fn get_encrypted_attachment_path(&self, uuid: &str) -> PathBuf {
        self.attachments_dir.join(format!("{}.enc", uuid))
    }

    pub fn save_chunk(&self, uuid: &str, chunk: &[u8], append: bool) -> Result<(), String> {
        let path = self.get_attachment_path(uuid);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(append)
            .truncate(!append)
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
            
        file.write_all(chunk).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_attachment(&self, uuid: &str) -> Result<(), String> {
        let path = self.get_attachment_path(uuid);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let legacy_path = self.get_legacy_attachment_path(uuid);
        if legacy_path.exists() {
            fs::remove_file(&legacy_path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn read_attachment_stream<F>(&self, uuid: &str, mut callback: F) -> Result<(), String>
    where
        F: FnMut(&[u8]) -> Result<(), String>,
    {
        let mut path = self.get_attachment_path(uuid);
        if !path.exists() {
            let legacy_path = self.get_legacy_attachment_path(uuid);
            if legacy_path.exists() {
                path = legacy_path;
            }
        }
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 65536]; // 64KB chunks
        
        loop {
            let bytes_read = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if bytes_read == 0 {
                break;
            }
            callback(&buffer[..bytes_read])?;
        }
        
        Ok(())
    }
}
