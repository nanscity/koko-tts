use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardMode {
    AutoCopy,
    DirectRead,
}

impl Default for ClipboardMode {
    fn default() -> Self {
        ClipboardMode::AutoCopy
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub hotkey: String,
    pub clipboard_mode: ClipboardMode,
    pub dont_restore_clipboard: bool,
    pub start_with_windows: bool,
    #[serde(default = "default_true")]
    pub clean_text: bool,
    pub voice: String,
    pub speed: f32,
    pub port: u16,
    #[serde(default)]
    pub online_api_key: String,
    #[serde(default)]
    pub online_endpoint: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+R".to_string(),
            clipboard_mode: ClipboardMode::AutoCopy,
            dont_restore_clipboard: false,
            start_with_windows: false,
            clean_text: true,
            voice: "af_bella".to_string(),
            speed: 1.0,
            port: 8787,
            online_api_key: String::new(),
            online_endpoint: String::new(),
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(appdata).join("KokoroTTS");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&data) {
                return settings;
            }
        }
        let default_settings = Self::default();
        let _ = default_settings.save();
        default_settings
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(())
    }
}
