/// L1 数据加密模块（服务端）
///
/// 提供 API Key 的 AES-256-GCM 加密存储。
/// 密钥管理：首次启动时生成随机 256-bit 密钥，存储在 %APPDATA%\LoongRecall\.lrc_key。
/// 与桌面端 crypto.rs 共享同一密钥文件，确保跨组件互操作。
///
/// 加密格式：Base64(Nonce[12B] || Ciphertext[变长] || Tag[16B])
///
/// 安全级别：L1（数据隐私层）
/// 契约：encrypt_api_key / decrypt_api_key 对外暴露，内部管理密钥生命周期。
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use std::path::PathBuf;

/// 密钥文件路径
fn key_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("LoongRecall").join(".lrc_key")
}

/// 获取或生成加密密钥（256-bit）
///
/// 首次调用时生成随机密钥并持久化到磁盘。
/// 后续调用从磁盘读取已有密钥。
/// 与桌面端共享同一密钥文件。
fn get_or_create_key() -> Result<[u8; 32], String> {
    let path = key_path();

    // 尝试读取已有密钥
    if path.exists() {
        let key_bytes = std::fs::read(&path).map_err(|e| format!("读取密钥文件失败: {e}"))?;
        if key_bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&key_bytes);
            return Ok(key);
        }
        // 密钥文件损坏，重新生成
        eprintln!("[加密] 密钥文件长度异常 ({}B)，重新生成", key_bytes.len());
    }

    // 生成新密钥
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);

    // 确保目录存在
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {e}"))?;
    }

    // 写入密钥文件
    std::fs::write(&path, key).map_err(|e| format!("写入密钥文件失败: {e}"))?;

    eprintln!("[加密] 已生成新加密密钥 (path={})", path.display());
    Ok(key)
}

/// 加密 API Key 字符串
///
/// 使用 AES-256-GCM 加密，随机生成 96-bit nonce。
/// 返回 Base64 编码的密文（nonce + ciphertext + tag）。
/// 空字符串返回空字符串（不加密空内容）。
pub fn encrypt_api_key(plaintext: &str) -> Result<String, String> {
    // 空字符串不加密（Ollama 等场景不需要 Key）
    if plaintext.is_empty() {
        return Ok(String::new());
    }

    let key = get_or_create_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建加密器失败: {e}"))?;

    // 生成随机 nonce（96-bit / 12 bytes）
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 加密
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("加密失败: {e}"))?;

    // 格式：nonce(12B) || ciphertext+tag(变长)
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(&combined))
}

/// 解密 API Key 字符串
///
/// 输入 Base64 编码的密文，返回明文 API Key。
/// 空字符串返回 Ok("")（未配置 Key）。
pub fn decrypt_api_key(encrypted: &str) -> Result<String, String> {
    // 空密文 = 未配置 Key
    if encrypted.is_empty() {
        return Ok(String::new());
    }
    let key = get_or_create_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建解密器失败: {e}"))?;

    // 解码 Base64
    let combined = BASE64
        .decode(encrypted)
        .map_err(|e| format!("Base64 解码失败: {e}"))?;

    if combined.len() < 12 + 16 {
        // 至少需要 nonce(12B) + tag(16B)
        return Err("密文数据不完整".into());
    }

    // 分离 nonce 和 ciphertext
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    // 解密
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("解密失败: {e}（密钥可能已变更）"))?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 解码失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：加密后解密应得到原始明文
    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let original = "sk-test-api-key-12345678";
        let encrypted = encrypt_api_key(original).expect("加密失败");
        // 加密后不应包含原始明文
        assert!(!encrypted.contains(original), "密文不应包含明文");
        // 解密后应恢复
        let decrypted = decrypt_api_key(&encrypted).expect("解密失败");
        assert_eq!(decrypted, original);
    }

    /// TDD：空字符串加密解密
    #[test]
    fn test_encrypt_decrypt_empty() {
        let original = "";
        let encrypted = encrypt_api_key(original).expect("加密失败");
        assert!(encrypted.is_empty(), "空字符串应返回空密文");
        let decrypted = decrypt_api_key(&encrypted).expect("解密失败");
        assert_eq!(decrypted, original);
    }

    /// TDD：错误密文应返回错误
    #[test]
    fn test_decrypt_invalid_data() {
        let result = decrypt_api_key("invalid-base64!!!");
        assert!(result.is_err(), "无效密文应返回错误");
    }

    /// TDD：密钥持久化
    #[test]
    fn test_key_persistence() {
        let original = "persistent-test-key";
        let encrypted = encrypt_api_key(original).expect("加密失败");
        let decrypted = decrypt_api_key(&encrypted).expect("解密失败");
        assert_eq!(decrypted, original);
    }
}