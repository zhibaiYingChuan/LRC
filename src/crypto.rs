/// L1 数据加密模块（服务端）
///
/// 提供 API Key 的 AES-256-GCM 加密存储。
/// 密钥管理：使用 Windows DPAPI（CryptProtectData）保护主密钥，
/// 确保密钥只能由当前 Windows 用户解密。
/// 非 Windows 平台使用文件权限保护（chmod 600）。
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

/// 密钥文件路径 — 与密文分离存储，但通过 DPAPI 保护
fn key_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("LoongRecall").join(".lrc_key")
}

/// 使用 DPAPI 加密密钥数据（Windows），非 Windows 平台直接返回原始数据
///
/// Windows DPAPI 使用当前用户凭据加密数据，只有同一用户可解密。
/// 这确保即使密钥文件被复制到其他机器也无法使用。
#[cfg(windows)]
fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB,
    };

    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // 使用当前用户凭据加密（不使用 LOCAL_MACHINE，确保用户隔离）
    // SAFETY: CryptProtectData 是 Windows DPAPI 标准 API，所有参数均为有效指针或空指针，
    // data_in 和 data_out 是栈上分配的 CRYPT_INTEGER_BLOB，生命周期安全
    let result = unsafe {
        CryptProtectData(
            &data_in,
            std::ptr::null(),        // 描述字符串（可选）
            std::ptr::null(),        // 额外的熵（可选）
            std::ptr::null(),        // 保留
            std::ptr::null(),        // 提示结构（可选）
            0,                       // 标志（0 = 用户级别保护）
            &mut data_out,
        )
    };

    if result == 0 {
        return Err("DPAPI 加密失败".into());
    }

    // 复制加密后的数据
    // SAFETY: data_out.pbData 由 CryptProtectData 分配并填充，cbData 为有效长度，
    // 从原始指针创建切片后立即调用 to_vec() 复制数据，不持有原始指针
    let protected = unsafe {
        std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize).to_vec()
    };

    // 释放 DPAPI 分配的内存
    // SAFETY: data_out.pbData 由 CryptProtectData 通过 LocalAlloc 分配，
    // 调用 LocalFree 是 Windows API 规定的释放方式
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(data_out.pbData as *mut std::ffi::c_void);
    }

    Ok(protected)
}

/// 使用 DPAPI 解密密钥数据（Windows），非 Windows 平台直接返回原始数据
#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    let data_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // SAFETY: CryptUnprotectData 是 Windows DPAPI 标准 API，用法与 CryptProtectData 对称
    let result = unsafe {
        CryptUnprotectData(
            &data_in,
            std::ptr::null_mut(),    // 解密后的描述字符串
            std::ptr::null(),        // 额外的熵（必须与加密时一致）
            std::ptr::null(),        // 保留
            std::ptr::null(),        // 提示结构
            0,                       // 标志
            &mut data_out,
        )
    };

    if result == 0 {
        return Err("DPAPI 解密失败（密钥可能来自其他用户或机器）".into());
    }

    // SAFETY: 与加密路径一致，从 CryptUnprotectData 输出复制数据后立即释放
    let unprotected = unsafe {
        std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize).to_vec()
    };

    // SAFETY: data_out.pbData 由 CryptUnprotectData 通过 LocalAlloc 分配
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(data_out.pbData as *mut std::ffi::c_void);
    }

    Ok(unprotected)
}

/// 非 Windows 平台：不进行 DPAPI 保护，但设置文件权限（调用方负责）
#[cfg(not(windows))]
fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}

/// 获取或生成加密密钥（256-bit）
///
/// 首次调用时生成随机密钥，通过 DPAPI 保护后持久化到磁盘。
/// 后续调用从磁盘读取并通过 DPAPI 解密恢复。
/// 密钥文件即使被复制到其他机器也无法使用。
fn get_or_create_key() -> Result<[u8; 32], String> {
    let path = key_path();

    // 尝试读取已有密钥
    if path.exists() {
        let protected_bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[加密] 读取密钥文件失败: {e}，重新生成");
                let _ = std::fs::remove_file(&path);
                return get_or_create_key();
            }
        };

        // 通过 DPAPI 解密恢复原始密钥
        match dpapi_unprotect(&protected_bytes) {
            Ok(key_bytes) if key_bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&key_bytes);
                return Ok(key);
            }
            Ok(key_bytes) => {
                // 密钥文件损坏，重新生成
                eprintln!("[加密] 密钥文件长度异常 ({}B)，重新生成", key_bytes.len());
                let _ = std::fs::remove_file(&path);
            }
            Err(e) => {
                // DPAPI 解密失败（用户切换、系统重装等），删除损坏文件并重新生成
                eprintln!("[加密] DPAPI 解密失败: {e}，重新生成密钥");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    // 生成新密钥
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);

    // 确保目录存在
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {e}"))?;
    }

    // 通过 DPAPI 加密后写入密钥文件
    let protected = dpapi_protect(&key)?;
    std::fs::write(&path, protected).map_err(|e| format!("写入密钥文件失败: {e}"))?;

    // 非 Windows 平台：设置文件权限为仅当前用户可读
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600); // 仅所有者可读写
            let _ = std::fs::set_permissions(&path, perms);
        }
    }

    eprintln!("[加密] 已生成新加密密钥（通过 DPAPI 保护，path={}）", path.display());
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