// crates/hermess-agent/src/crypto.rs
// AES-256-GCM 加密层：加密会话数据、evolution 状态及其他敏感持久化数据。
//
// 密钥管理：
//   - 优先从 HERMES_ENCRYPTION_KEY 环境变量读取（64 位 hex = 32 字节）
//   - 否则从 ~/.hermes/encryption.key 读取
//   - 否则自动生成并持久化到 ~/.hermes/encryption.key
//
// 使用方式：
//   let vault = DataVault::load_or_create()?;
//   let encrypted = vault.encrypt(b"my secret data")?;
//   let decrypted = vault.decrypt(&encrypted)?;

use anyhow::Context;
use std::path::PathBuf;

/// AES-256-GCM nonce size (96 bits = 12 bytes).
const NONCE_SIZE: usize = 12;
/// AES-256 key size (256 bits = 32 bytes).
const KEY_SIZE: usize = 32;

/// Encrypted data vault using AES-256-GCM.
///
/// Each call to `encrypt()` generates a fresh random nonce, so identical
/// plaintexts produce different ciphertexts.
pub struct DataVault {
    key: [u8; KEY_SIZE],
}

impl DataVault {
    /// Load key from env var, file, or generate a new one.
    pub fn load_or_create() -> anyhow::Result<Self> {
        // 1. Try environment variable
        if let Ok(hex_key) = std::env::var("HERMES_ENCRYPTION_KEY") {
            if let Some(key) = Self::parse_hex_key(&hex_key) {
                tracing::info!("Encryption key loaded from HERMES_ENCRYPTION_KEY");
                return Ok(Self { key });
            }
            tracing::warn!("HERMES_ENCRYPTION_KEY is set but invalid (expected 64 hex chars)");
        }

        // 2. Try key file
        let key_path = Self::key_path();
        if key_path.exists() {
            let hex_key = std::fs::read_to_string(&key_path)
                .with_context(|| format!("Failed to read {}", key_path.display()))?;
            let hex_key = hex_key.trim();
            if let Some(key) = Self::parse_hex_key(hex_key) {
                tracing::info!("Encryption key loaded from {}", key_path.display());
                return Ok(Self { key });
            }
            tracing::warn!("Key file corrupt, regenerating");
        }

        // 3. Generate new key
        let key = Self::generate_key();
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let hex = hex::encode(key);
        std::fs::write(&key_path, &hex)
            .with_context(|| format!("Failed to write key to {}", key_path.display()))?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&key_path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&key_path, perms);
            }
        }

        tracing::info!("New encryption key generated at {}", key_path.display());
        Ok(Self { key })
    }

    /// Encrypt plaintext with AES-256-GCM. Returns (nonce || ciphertext).
    pub fn encrypt(&self, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
        use aes_gcm::{
            aead::{Aead, KeyInit, OsRng},
            Aes256Gcm, Nonce,
        };
        use rand::RngCore;

        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| anyhow::anyhow!("AES init: {e}"))?;

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;

        // Prepend nonce to ciphertext
        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext produced by `encrypt()`. Input format: (nonce || ciphertext).
    pub fn decrypt(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        use aes_gcm::{
            aead::{Aead, KeyInit},
            Aes256Gcm, Nonce,
        };

        if data.len() < NONCE_SIZE + 16 {
            anyhow::bail!("Ciphertext too short ({} bytes)", data.len());
        }

        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| anyhow::anyhow!("AES init: {e}"))?;

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Decryption failed — wrong key or corrupted data: {e}"))
    }

    /// Encrypt a string and return base64-encoded result. Convenience for JSON/DB storage.
    pub fn encrypt_str(&self, plaintext: &str) -> anyhow::Result<String> {
        let encrypted = self.encrypt(plaintext.as_bytes())?;
        Ok(base64_encode(&encrypted))
    }

    /// Decrypt a base64-encoded string produced by `encrypt_str()`.
    pub fn decrypt_str(&self, b64: &str) -> anyhow::Result<String> {
        let data = base64_decode(b64)?;
        let decrypted = self.decrypt(&data)?;
        String::from_utf8(decrypted).map_err(|e| anyhow::anyhow!("Invalid UTF-8: {e}"))
    }

    fn key_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".hermes").join("encryption.key")
    }

    fn parse_hex_key(hex: &str) -> Option<[u8; KEY_SIZE]> {
        let hex = hex.trim();
        if hex.len() != KEY_SIZE * 2 {
            return None;
        }
        let bytes = hex::decode(hex).ok()?;
        let mut key = [0u8; KEY_SIZE];
        key.copy_from_slice(&bytes);
        Some(key)
    }

    fn generate_key() -> [u8; KEY_SIZE] {
        use rand::RngCore;
        let mut key = [0u8; KEY_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut key);
        key
    }
}

/// Base64-encode bytes (URL-safe, no padding).
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Base64-decode (URL-safe, no padding).
fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| anyhow::anyhow!("Base64 decode failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let plaintext = b"hello, this is secret session data";
        let encrypted = vault.encrypt(plaintext).unwrap();
        assert_ne!(&encrypted[..plaintext.len()], plaintext);
        let decrypted = vault.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_str_roundtrip() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let original = "用户会话数据: user_id=abc123, token=sensitive";
        let encrypted = vault.encrypt_str(original).unwrap();
        assert!(!encrypted.contains("abc123"));
        let decrypted = vault.decrypt_str(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn nonce_uniqueness() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let e1 = vault.encrypt(b"same data").unwrap();
        let e2 = vault.encrypt(b"same data").unwrap();
        assert_ne!(
            e1, e2,
            "Same plaintext should produce different ciphertexts"
        );
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let v1 = DataVault {
            key: DataVault::generate_key(),
        };
        let v2 = DataVault {
            key: DataVault::generate_key(),
        };
        let encrypted = v1.encrypt(b"secret").unwrap();
        assert!(v2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn corrupted_data_fails() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let mut encrypted = vault.encrypt(b"data").unwrap();
        // Flip a bit in the ciphertext
        if let Some(b) = encrypted.last_mut() {
            *b ^= 1;
        }
        assert!(vault.decrypt(&encrypted).is_err());
    }

    #[test]
    fn empty_plaintext() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let encrypted = vault.encrypt(b"").unwrap();
        let decrypted = vault.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn large_plaintext() {
        let vault = DataVault {
            key: DataVault::generate_key(),
        };
        let data = vec![0xAA; 1024 * 1024]; // 1MB
        let encrypted = vault.encrypt(&data).unwrap();
        let decrypted = vault.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn parse_valid_hex_key() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let key = DataVault::parse_hex_key(hex);
        assert!(key.is_some());
    }

    #[test]
    fn parse_invalid_hex_key() {
        assert!(DataVault::parse_hex_key("short").is_none());
        assert!(DataVault::parse_hex_key("gg").is_none());
    }
}
