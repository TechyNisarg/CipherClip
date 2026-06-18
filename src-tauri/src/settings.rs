use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub history_limit: i64,
    pub global_shortcut: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub master_password_hash: Option<String>,
    #[serde(default)]
    pub blocked_ips: Vec<String>,
}

fn default_theme() -> String {
    "system".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            history_limit: 100, // Default limit
            global_shortcut: "CommandOrControl+Shift+C".to_string(),
            theme: default_theme(),
            master_password_hash: None,
            blocked_ips: Vec::new(),
        }
    }
}

pub struct SettingsManager {
    path: PathBuf,
    settings: Arc<Mutex<AppSettings>>,
}

impl SettingsManager {
    pub fn new(app_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&app_dir).unwrap_or_default();
        let path = app_dir.join("settings.json");

        let settings = if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                AppSettings::default()
            }
        } else {
            AppSettings::default()
        };

        // Save immediately in case it didn't exist or was invalid
        let manager = Self {
            path,
            settings: Arc::new(Mutex::new(settings)),
        };
        manager.save().unwrap_or_default();

        manager
    }

    pub fn get(&self) -> AppSettings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_limit(&self, limit: i64) -> Result<(), String> {
        {
            let mut s = self.settings.lock().unwrap();
            s.history_limit = limit;
        }
        self.save().map_err(|e| e.to_string())
    }

    pub fn get_blocked_ips(&self) -> Vec<String> {
        self.settings.lock().unwrap().blocked_ips.clone()
    }

    pub fn add_blocked_ip(&self, ip: String) {
        let mut s = self.settings.lock().unwrap();
        if !s.blocked_ips.contains(&ip) {
            s.blocked_ips.push(ip);
            drop(s);
            let _ = self.save();
        }
    }

    pub fn clear_blocked_ips(&self) {
        let mut s = self.settings.lock().unwrap();
        s.blocked_ips.clear();
        drop(s);
        let _ = self.save();
    }

    pub fn set_shortcut(&self, shortcut: String) -> Result<(), String> {
        {
            let mut s = self.settings.lock().unwrap();
            s.global_shortcut = shortcut;
        }
        self.save().map_err(|e| e.to_string())
    }

    pub fn set_theme(&self, theme: String) -> Result<(), String> {
        {
            let mut s = self.settings.lock().unwrap();
            s.theme = theme;
        }
        self.save().map_err(|e| e.to_string())
    }

    pub fn set_master_password_hash(&self, hash: Option<String>) -> Result<(), String> {
        {
            let mut s = self.settings.lock().unwrap();
            s.master_password_hash = hash;
        }
        self.save().map_err(|e| e.to_string())
    }

    fn save(&self) -> std::io::Result<()> {
        let s = self.settings.lock().unwrap();
        let json = serde_json::to_string_pretty(&*s)?;
        fs::write(&self.path, json)
    }
}
