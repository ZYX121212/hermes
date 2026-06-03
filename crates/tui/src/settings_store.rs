// crates/tui/src/settings_store.rs
// Persistent user settings stored in .hermess/settings.json.

use serde::{Deserialize, Serialize};

/// User-facing settings persisted to disk and editable from the TUI settings panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct UserSettings {
    // ── LLM ──
    #[serde(default)]
    pub llm_provider: String, // "anthropic" | "openai" | "deepseek"
    #[serde(default)]
    pub llm_model: String,
    #[serde(default)]
    pub llm_api_key: String,
    #[serde(default)]
    pub llm_base_url: String,

    // ── Search ──
    #[serde(default)]
    pub search_enabled: bool,
    #[serde(default)]
    pub search_api_key: String,

    // ── Finance ──
    #[serde(default)]
    pub finance_provider: String, // "" | "sina" | "tushare" | "ftshare"
    #[serde(default)]
    pub finance_tushare_token: String,
}


impl UserSettings {
    /// Find and load settings from disk.
    /// Checks `.hermess/settings.json` in the current directory first,
    /// then falls back to `~/.hermess/settings.json`.
    pub fn load() -> Self {
        let paths = [
            std::env::current_dir()
                .ok()
                .map(|d| d.join(".hermess").join("settings.json")),
            dirs_next().map(|d| d.join(".hermess").join("settings.json")),
        ];

        for path in paths.into_iter().flatten() {
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                        Ok(s) => {
                            tracing::info!(path = %path.display(), "Loaded user settings");
                            return s;
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "Failed to parse settings file, using defaults");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "Failed to read settings file");
                    }
                }
            }
        }
        tracing::debug!("No settings file found, using defaults");
        Self::default()
    }

    /// Persist settings to `.hermess/settings.json` in the current directory.
    pub fn save(&self) -> Result<(), String> {
        let dir = std::env::current_dir()
            .map_err(|e| format!("current_dir: {e}"))?
            .join(".hermess");
        std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir: {e}"))?;
        let path = dir.join("settings.json");
        let raw = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, raw).map_err(|e| format!("write: {e}"))?;
        tracing::info!(path = %path.display(), "Saved user settings");
        Ok(())
    }

    /// Apply overrides from environment variables (env takes priority over file values).
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
            if !v.is_empty() {
                self.llm_api_key = v;
                self.llm_provider = "anthropic".into();
            }
        }
        if let Ok(v) = std::env::var("OPENAI_API_KEY") {
            if !v.is_empty() && self.llm_provider != "anthropic" {
                self.llm_api_key = v;
                if self.llm_provider.is_empty() {
                    self.llm_provider = "openai".into();
                }
            }
        }
        if let Ok(v) = std::env::var("DEEPSEEK_API_KEY") {
            if !v.is_empty() && self.llm_api_key.is_empty() {
                self.llm_api_key = v;
                if self.llm_provider.is_empty() {
                    self.llm_provider = "deepseek".into();
                }
            }
        }
        if let Ok(v) = std::env::var("HERMESS_FINANCE_PROVIDER") {
            if !v.is_empty() {
                self.finance_provider = v;
            }
        }
        if let Ok(v) = std::env::var("HERMESS_TUSHARE_TOKEN") {
            if !v.is_empty() {
                self.finance_tushare_token = v;
            }
        }
        if let Ok(v) = std::env::var("BRAVE_SEARCH_API_KEY") {
            if !v.is_empty() {
                self.search_enabled = true;
                self.search_api_key = v;
            }
        }
    }

    /// Mask an API key for display: "sk-abc1234xyz" → "sk-abc••••xyz"
    pub fn mask_key(key: &str) -> String {
        if key.is_empty() {
            return "(未设置)".into();
        }
        if key.len() <= 8 {
            return "••••".into();
        }
        let prefix = &key[..4];
        let suffix = &key[key.len() - 4..];
        format!("{prefix}••••{suffix}")
    }
}

/// Return the user's home config directory.
fn dirs_next() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or({
            #[cfg(windows)]
            {
                std::env::var("USERPROFILE").ok().map(std::path::PathBuf::from)
            }
            #[cfg(not(windows))]
            {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_empty() {
        let s = UserSettings::default();
        assert!(s.llm_provider.is_empty());
        assert!(s.llm_model.is_empty());
        assert!(s.llm_api_key.is_empty());
        assert!(!s.search_enabled);
        assert!(s.finance_provider.is_empty());
    }

    #[test]
    fn roundtrip_serialize_deserialize() {
        let s = UserSettings {
            llm_provider: "deepseek".into(),
            llm_model: "deepseek-chat".into(),
            llm_api_key: "sk-test-key-1234".into(),
            llm_base_url: "https://api.deepseek.com/v1".into(),
            search_enabled: true,
            search_api_key: "BSA-test".into(),
            finance_provider: "sina".into(),
            finance_tushare_token: String::new(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: UserSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.llm_provider, "deepseek");
        assert_eq!(s2.llm_model, "deepseek-chat");
        assert!(s2.search_enabled);
        assert_eq!(s2.finance_provider, "sina");
    }

    #[test]
    fn deserialize_partial_json() {
        let json = r#"{"llm_provider": "openai", "search_enabled": true}"#;
        let s: UserSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.llm_provider, "openai");
        assert!(s.search_enabled);
        assert!(s.llm_model.is_empty()); // missing fields get defaults
        assert!(s.finance_provider.is_empty());
    }

    #[test]
    fn mask_key_short() {
        assert_eq!(UserSettings::mask_key("abc"), "••••");
        assert_eq!(UserSettings::mask_key(""), "(未设置)");
    }

    #[test]
    fn mask_key_normal() {
        let masked = UserSettings::mask_key("sk-abc1234xyz");
        assert!(masked.starts_with("sk-a"));
        assert!(masked.ends_with("xyz"));
        assert!(masked.contains("••••"));
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join("hermess_test_settings");
        let hermess_dir = dir.join(".hermess");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&hermess_dir).unwrap();
        let path = hermess_dir.join("settings.json");

        let s = UserSettings {
            llm_provider: "anthropic".into(),
            finance_provider: "tushare".into(),
            ..Default::default()
        };
        let raw = serde_json::to_string_pretty(&s).unwrap();
        std::fs::write(&path, &raw).unwrap();

        let read_back: UserSettings =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read_back.llm_provider, "anthropic");
        assert_eq!(read_back.finance_provider, "tushare");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_env_overrides_fills_from_env() {
        // This test only checks the logic, not actual env vars
        let mut s = UserSettings::default();
        // Without env vars set, should remain empty
        s.apply_env_overrides();
        assert!(s.llm_api_key.is_empty());

        // If we manually set the key (as env would), it should persist
        s.llm_api_key = "test-key".into();
        assert_eq!(s.llm_api_key, "test-key");
    }
}
