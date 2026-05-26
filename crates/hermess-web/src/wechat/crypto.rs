// 企业微信回调消息加解密
// 参考: https://developer.work.weixin.qq.com/document/path/90968

use aes::cipher::{block_padding::Pkcs7, generic_array::GenericArray, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use anyhow::{bail, Context as _};
use base64::Engine;
use rand::Rng;
use sha1::{Digest, Sha1};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// 验证回调 URL 签名
pub fn verify_signature(
    token: &str,
    timestamp: &str,
    nonce: &str,
    msg_encrypt: &str,
    msg_signature: &str,
) -> bool {
    let mut parts = [token, timestamp, nonce, msg_encrypt];
    parts.sort();
    let combined = parts.join("");
    let mut hasher = Sha1::new();
    hasher.update(combined.as_bytes());
    let digest = hasher.finalize();
    let sig = hex::encode(digest);
    sig == msg_signature
}

/// 解密企业微信回调消息
pub fn decrypt_msg(encrypted: &str, encoding_aes_key: &str) -> anyhow::Result<String> {
    let key = decode_aes_key(encoding_aes_key)?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(encrypted.as_bytes())
        .context("base64 decode failed")?;

    // AES-256-CBC: IV is the first 16 bytes of the key
    let key_arr = GenericArray::from_slice(&key);
    let iv_arr = GenericArray::from_slice(&key[..16]);

    let mut buf = ciphertext.clone();
    let decrypted = Aes256CbcDec::new(key_arr, iv_arr)
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("AES decrypt failed: {:?}", e))?;

    if decrypted.len() < 20 {
        bail!("decrypted payload too short");
    }

    let msg_len = u32::from_be_bytes([decrypted[16], decrypted[17], decrypted[18], decrypted[19]])
        as usize;

    if 20 + msg_len > decrypted.len() {
        bail!("invalid message length in decrypted payload");
    }

    let msg = String::from_utf8(decrypted[20..20 + msg_len].to_vec())
        .context("message is not valid UTF-8")?;

    Ok(msg)
}

/// 加密回复消息
pub fn encrypt_msg(
    plain: &str,
    encoding_aes_key: &str,
    corp_id: &str,
) -> anyhow::Result<String> {
    let key = decode_aes_key(encoding_aes_key)?;

    let mut rng = rand::thread_rng();
    let random: [u8; 16] = rng.gen();

    let msg_bytes = plain.as_bytes();
    let corp_bytes = corp_id.as_bytes();
    let total_len = 16 + 4 + msg_bytes.len() + corp_bytes.len();
    let mut buf = Vec::with_capacity(total_len);

    buf.extend_from_slice(&random);
    buf.extend_from_slice(&(msg_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(msg_bytes);
    buf.extend_from_slice(corp_bytes);

    let key_arr = GenericArray::from_slice(&key);
    let iv_arr = GenericArray::from_slice(&key[..16]);

    // encrypt_padded_mut needs buffer with space for padding
    let msg_len = buf.len();
    let block_size: usize = 16;
    let pad_len = block_size - (msg_len % block_size);
    buf.resize(msg_len + pad_len, 0);

    let encrypted = Aes256CbcEnc::new(key_arr, iv_arr)
        .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
        .map_err(|e| anyhow::anyhow!("AES encrypt failed: {:?}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

/// 生成回调回复的签名
pub fn sign_reply(
    token: &str,
    timestamp: &str,
    nonce: &str,
    encrypted: &str,
) -> String {
    let mut parts = [token, timestamp, nonce, encrypted];
    parts.sort();
    let combined = parts.join("");
    let mut hasher = Sha1::new();
    hasher.update(combined.as_bytes());
    hex::encode(hasher.finalize())
}

fn decode_aes_key(encoding_aes_key: &str) -> anyhow::Result<Vec<u8>> {
    let mut a = encoding_aes_key.to_string();
    if !a.ends_with('=') {
        a.push('=');
    }
    let key = base64::engine::general_purpose::STANDARD
        .decode(a.as_bytes())
        .context("invalid AES key encoding")?;
    if key.len() != 32 {
        bail!("AES key must be 32 bytes, got {}", key.len());
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decrypt_encrypt_roundtrip() {
        let key_bytes: [u8; 32] = rand::thread_rng().gen();
        let aes_key = base64::engine::general_purpose::STANDARD.encode(key_bytes);
        let aes_key = aes_key[..43].to_string();

        let corp_id = "ww123456";
        let plain = "<xml><ToUserName>corp</ToUserName><Content>hello</Content></xml>";

        let encrypted = encrypt_msg(plain, &aes_key, corp_id).unwrap();
        let decrypted = decrypt_msg(&encrypted, &aes_key).unwrap();

        assert_eq!(decrypted, plain);
    }

    #[test]
    fn test_signature() {
        let token = "test_token";
        let timestamp = "1409659589";
        let nonce = "263014780";
        let msg_encrypt = "encrypted_msg_example";

        let sig = sign_reply(token, timestamp, nonce, msg_encrypt);
        assert!(verify_signature(token, timestamp, nonce, msg_encrypt, &sig));
    }
}
