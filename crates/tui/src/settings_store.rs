// crates/tui/src/settings_store.rs
// Persistent user settings stored in .hermess/settings.json.

use serde::{Deserialize, Serialize};

/// User-facing settings persisted to disk and editable from the TUI settings panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub finance_provider: String, // "ftshare" | "tushare" | "sina" | "eastmoney" | "tencent"
    #[serde(default)]
    pub finance_tushare_token: String,

    // ── LiteLLM 模型目录 ──
    #[serde(default)]
    pub litellm_url: String,

    // ── Feishu ──
    #[serde(default)]
    pub feishu_app_id: String,
    #[serde(default)]
    pub feishu_app_secret: String,
    #[serde(default)]
    pub feishu_bot_open_id: String,

    // ── WeChat Work ──
    #[serde(default)]
    pub wechat_corp_id: String,
    #[serde(default)]
    pub wechat_corp_secret: String,
    #[serde(default)]
    pub wechat_agent_id: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            llm_provider: "deepseek".into(),
            llm_model: "deepseek-chat".into(),
            llm_base_url: "https://api.deepseek.com/v1".into(),
            llm_api_key: "sk-4ab52089feed4d788eee376dfaa4bbb3".into(),
            search_enabled: false,
            search_api_key: String::new(),
            finance_provider: "ftshare".into(),
            finance_tushare_token: String::new(),
            litellm_url: String::new(),
            feishu_app_id: String::new(),
            feishu_app_secret: String::new(),
            feishu_bot_open_id: String::new(),
            wechat_corp_id: String::new(),
            wechat_corp_secret: String::new(),
            wechat_agent_id: String::new(),
        }
    }
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
                        Ok(mut s) => {
                            s.fill_defaults();
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

        // 同步飞书配置到 config/feishu.toml
        if !self.feishu_app_id.is_empty() {
            let feishu_path = std::path::Path::new("config/feishu.toml");
            let feishu_content = format!(
                r#"# Hermes Web Daemon — 飞书配置
[feishu]
app_id = "{}"
app_secret = "{}"
bot_open_id = "{}"
"#,
                self.feishu_app_id, self.feishu_app_secret, self.feishu_bot_open_id
            );
            if let Some(parent) = feishu_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(feishu_path, &feishu_content) {
                tracing::warn!(error = %e, "Failed to sync feishu config to config/feishu.toml");
            } else {
                tracing::info!("Synced feishu config to config/feishu.toml");
            }
        }

        // 同步企业微信配置到 config/wechat.toml
        if !self.wechat_corp_id.is_empty() {
            let wechat_path = std::path::Path::new("config/wechat.toml");
            let wechat_content = format!(
                r#"# Hermes Web Daemon — 企业微信配置
[wechat]
corp_id = "{}"
secret = "{}"
agent_id = "{}"
"#,
                self.wechat_corp_id, self.wechat_corp_secret, self.wechat_agent_id
            );
            if let Some(parent) = wechat_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(wechat_path, &wechat_content) {
                tracing::warn!(error = %e, "Failed to sync wechat config to config/wechat.toml");
            } else {
                tracing::info!("Synced wechat config to config/wechat.toml");
            }
        }

        Ok(())
    }

    /// Fill empty fields from the struct default, so a sparse settings.json
    /// doesn't wipe out hardcoded defaults (e.g. the API key).
    fn fill_defaults(&mut self) {
        let defaults = Self::default();
        if self.llm_provider.is_empty() {
            self.llm_provider = defaults.llm_provider;
        }
        if self.llm_model.is_empty() {
            self.llm_model = defaults.llm_model;
        }
        if self.llm_api_key.is_empty() {
            self.llm_api_key = defaults.llm_api_key;
        }
        if self.llm_base_url.is_empty() {
            self.llm_base_url = defaults.llm_base_url;
        }
        if self.finance_provider.is_empty() {
            self.finance_provider = defaults.finance_provider;
        }
    }

    /// Apply overrides from environment variables (env takes priority over file values).
    /// Priority: DEEPSEEK_API_KEY > ANTHROPIC_API_KEY > OPENAI_API_KEY (deepseek is default).
    pub fn apply_env_overrides(&mut self) {
        // DeepSeek (default provider): always apply if available
        if let Ok(v) = std::env::var("DEEPSEEK_API_KEY") {
            if !v.is_empty() {
                self.llm_provider = "deepseek".into();
                self.llm_api_key = v;
                if self.llm_base_url.is_empty() {
                    self.llm_base_url = "https://api.deepseek.com/v1".into();
                }
                if self.llm_model.is_empty() {
                    self.llm_model = "deepseek-chat".into();
                }
            }
        }
        if let Ok(v) = std::env::var("OPENAI_API_KEY") {
            if !v.is_empty() && !v.is_empty() && self.llm_api_key.is_empty() {
                self.llm_api_key = v;
                if self.llm_provider.is_empty() {
                    self.llm_provider = "openai".into();
                }
            }
        }
        if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
            if !v.is_empty() && self.llm_api_key.is_empty() {
                self.llm_api_key = v;
                self.llm_provider = "anthropic".into();
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
        if let Ok(v) = std::env::var("FEISHU_APP_ID") {
            if !v.is_empty() {
                self.feishu_app_id = v;
            }
        }
        if let Ok(v) = std::env::var("FEISHU_APP_SECRET") {
            if !v.is_empty() {
                self.feishu_app_secret = v;
            }
        }
        if let Ok(v) = std::env::var("FEISHU_BOT_OPEN_ID") {
            if !v.is_empty() {
                self.feishu_bot_open_id = v;
            }
        }
        if let Ok(v) = std::env::var("WECHAT_CORP_ID") {
            if !v.is_empty() {
                self.wechat_corp_id = v;
            }
        }
        if let Ok(v) = std::env::var("WECHAT_CORP_SECRET") {
            if !v.is_empty() {
                self.wechat_corp_secret = v;
            }
        }
        if let Ok(v) = std::env::var("WECHAT_AGENT_ID") {
            if !v.is_empty() {
                self.wechat_agent_id = v;
            }
        }
        if let Ok(v) = std::env::var("LITELLM_URL") {
            if !v.is_empty() {
                self.litellm_url = v;
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
                std::env::var("USERPROFILE")
                    .ok()
                    .map(std::path::PathBuf::from)
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
    fn default_settings_are_deepseek() {
        let s = UserSettings::default();
        assert_eq!(s.llm_provider, "deepseek");
        assert_eq!(s.llm_model, "deepseek-chat");
        assert!(!s.llm_api_key.is_empty()); // default api key set
        assert!(!s.search_enabled);
        assert_eq!(s.finance_provider, "ftshare");
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
            feishu_app_id: "cli_test".into(),
            feishu_app_secret: "secret_test".into(),
            feishu_bot_open_id: "bot_ou_001".into(),
            litellm_url: "http://localhost:4000".into(),
            wechat_corp_id: "ww_test".into(),
            wechat_corp_secret: "secret_wx".into(),
            wechat_agent_id: "1000002".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: UserSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.llm_provider, "deepseek");
        assert_eq!(s2.llm_model, "deepseek-chat");
        assert!(s2.search_enabled);
        assert_eq!(s2.finance_provider, "sina");
        assert_eq!(s2.feishu_app_id, "cli_test");
        assert_eq!(s2.feishu_bot_open_id, "bot_ou_001");
        assert_eq!(s2.wechat_corp_id, "ww_test");
        assert_eq!(s2.wechat_agent_id, "1000002");
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
    fn apply_env_overrides_preserves_default_key() {
        // This test only checks the logic, not actual env vars
        let mut s = UserSettings::default();
        let default_key = s.llm_api_key.clone();
        // Without env vars set, default api_key is preserved
        s.apply_env_overrides();
        assert_eq!(s.llm_api_key, default_key);

        // Provider should remain deepseek
        assert_eq!(s.llm_provider, "deepseek");
    }

    #[test]
    fn fill_defaults_restores_empty_fields() {
        // Simulate loading from a sparse JSON where key is empty
        let json = r#"{"llm_provider": "", "llm_api_key": "", "search_enabled": true}"#;
        let mut s: UserSettings = serde_json::from_str(json).unwrap();
        s.fill_defaults();
        assert_eq!(s.llm_provider, "deepseek");
        assert_eq!(s.llm_model, "deepseek-chat");
        assert_eq!(s.llm_api_key, "sk-4ab52089feed4d788eee376dfaa4bbb3");
        assert_eq!(s.llm_base_url, "https://api.deepseek.com/v1");
        assert_eq!(s.finance_provider, "ftshare");
        assert!(s.search_enabled); // explicit true preserved
    }

    #[test]
    fn fill_defaults_restores_ftshare_finance_provider() {
        let json = r#"{"finance_provider": ""}"#;
        let mut s: UserSettings = serde_json::from_str(json).unwrap();
        s.fill_defaults();
        assert_eq!(s.finance_provider, "ftshare");
    }
}
