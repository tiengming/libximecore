//! 插件配置值加密（对齐 Android `SecureValueCipher` 的设计与密文格式）：
//! - 算法 AES-256-GCM；密文格式 `enc:` + base64(iv(12B) + 密文+tag(16B))
//! - 解密：无 `enc:` 前缀 → 原样返回（旧版明文兼容）；GCM 认证失败 → None
//! - 密钥：随机 32 字节，平台原语保护——Windows 经 DPAPI(CryptProtectData)
//!   加密后存 `secret.key`；Linux 存 Secret Service（org.freedesktop.se，
//!   KDE Wallet / GNOME Keyring 均实现，跨进程共享），无 Secret Service 的
//!   环境降级明文直通（与旧版行为一致，不阻塞配置读写）
//! - 两端密文格式逐字节一致（与 Android 互换）
//!
//! 用法：[`encrypt_with_key_path`] 写入、[`decrypt_with_key_path`] 读取；
//! `key_path` 由调用方从配置文件路径推导（Windows 用；Linux 密钥在 keyring，
//! 参数仅保留签名兼容）。

use std::path::{Path, PathBuf};

/// 密文前缀（与 Android `SecureValueCipher` 一致）。
const ENC_PREFIX: &str = "enc:";

/// GCM nonce 长度（字节）。
const NONCE_LEN: usize = 12;

/// AES-256 密钥长度（字节）。
const KEY_LEN: usize = 32;

/// 加密：`enc:` + base64(iv + ciphertext)。密钥不可用时降级明文原样返回
/// （不能让插件配置写入失败）。
pub fn encrypt_with_key_path(key_path: &Path, plain: &str) -> String {
    #[cfg(windows)]
    {
        imp::encrypt(key_path, plain).unwrap_or_else(|_| plain.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = key_path;
        imp::encrypt(plain).unwrap_or_else(|_| plain.to_string())
    }
}

/// 解密：`enc:` 前缀 → base64 解码 + AES-GCM 认证解密（失败返回 None，
/// 对齐 Android 认证失败视为无效）；无前缀 → 原样返回（旧版明文兼容）。
pub fn decrypt_with_key_path(key_path: &Path, stored: &str) -> Option<String> {
    #[cfg(windows)]
    {
        imp::decrypt(key_path, stored)
    }
    #[cfg(not(windows))]
    {
        let _ = key_path;
        imp::decrypt(stored)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Nonce};

    // ---- DPAPI（crypt32.dll）最小 FFI ----

    #[repr(C)]
    struct CryptoBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const CryptoBlob,
            description: *const u16,
            entropy: *const CryptoBlob,
            reserved: *const core::ffi::c_void,
            prompt: *const core::ffi::c_void,
            flags: u32,
            data_out: *mut CryptoBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const CryptoBlob,
            description: *mut *mut u16,
            entropy: *const CryptoBlob,
            reserved: *const core::ffi::c_void,
            prompt: *const core::ffi::c_void,
            flags: u32,
            data_out: *mut CryptoBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    /// DPAPI 加密（用户作用域，默认熵）。
    fn dpapi_protect(plain: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let mut blob_in = CryptoBlob {
                cb_data: plain.len() as u32,
                pb_data: plain.as_ptr() as *mut u8,
            };
            let mut blob_out = CryptoBlob {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            };
            if CryptProtectData(
                &mut blob_in,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut blob_out,
            ) == 0
                || blob_out.pb_data.is_null()
            {
                return None;
            }
            let out =
                std::slice::from_raw_parts(blob_out.pb_data, blob_out.cb_data as usize).to_vec();
            LocalFree(blob_out.pb_data as *mut _);
            Some(out)
        }
    }

    /// DPAPI 解密。
    fn dpapi_unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let mut blob_in = CryptoBlob {
                cb_data: blob.len() as u32,
                pb_data: blob.as_ptr() as *mut u8,
            };
            let mut blob_out = CryptoBlob {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            };
            if CryptUnprotectData(
                &mut blob_in,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut blob_out,
            ) == 0
                || blob_out.pb_data.is_null()
            {
                return None;
            }
            let out =
                std::slice::from_raw_parts(blob_out.pb_data, blob_out.cb_data as usize).to_vec();
            LocalFree(blob_out.pb_data as *mut _);
            Some(out)
        }
    }

    /// 加载（或首次生成并落盘）AES-256 主密钥：文件内容为
    /// base64(DPAPI(32 字节随机密钥))，密钥本体不落明文。
    fn load_or_create_key(key_path: &Path) -> Option<[u8; KEY_LEN]> {
        if let Ok(content) = std::fs::read_to_string(key_path) {
            let content = content.trim();
            if let Some(blob) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content)
                    .ok()
                    .and_then(|b| dpapi_unprotect(&b))
            {
                if blob.len() == KEY_LEN {
                    let mut key = [0u8; KEY_LEN];
                    key.copy_from_slice(&blob);
                    return Some(key);
                }
            }
            // 密钥文件损坏/属于其他用户：不覆盖（避免误清既有密文），走解密失败路径
            return None;
        }
        // 首次生成
        let mut key = [0u8; KEY_LEN];
        getrandom::fill(&mut key).ok()?;
        if let Some(blob) = dpapi_protect(&key) {
            if let Some(parent) = key_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, blob);
            if std::fs::write(key_path, encoded).is_ok() {
                return Some(key);
            }
        }
        None
    }

    pub fn encrypt(key_path: &Path, plain: &str) -> Result<String, &'static str> {
        let key = load_or_create_key(key_path).ok_or("密钥不可用")?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "密钥长度错误")?;
        let mut iv = [0u8; NONCE_LEN];
        getrandom::fill(&mut iv).map_err(|_| "随机数不可用")?;
        // Nonce::from_slice 在 aes-gcm 0.11 已弃用：改用 copy_from_slice 填充。
        let mut nonce = Nonce::default();
        nonce.copy_from_slice(&iv);
        let ct = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plain.as_bytes(),
                    aad: &[],
                },
            )
            .map_err(|_| "加密失败")?;
        let mut out = iv.to_vec();
        out.extend_from_slice(&ct);
        Ok(format!(
            "{}{}",
            ENC_PREFIX,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, out)
        ))
    }

    pub fn decrypt(key_path: &Path, stored: &str) -> Option<String> {
        let Some(encoded) = stored.strip_prefix(ENC_PREFIX) else {
            // 无前缀：旧版明文兼容
            return Some(stored.to_string());
        };
        let blob =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
        if blob.len() <= NONCE_LEN {
            return None;
        }
        let (iv, ct) = blob.split_at(NONCE_LEN);
        let key = load_or_create_key(key_path)?;
        let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
        let mut nonce = Nonce::default();
        nonce.copy_from_slice(iv);
        let plain = cipher
            .decrypt(&nonce, Payload { msg: ct, aad: &[] })
            .ok()?;
        String::from_utf8(plain).ok()
    }
}

/// Linux：主密钥存 Secret Service（keyring，service=`xime` / user=`plugin-cipher`，
/// KDE Wallet / GNOME Keyring 均实现，daemon 与设置进程跨进程共享）。
/// AES-GCM 与密文格式同 Windows 一套常量。key_path 参数忽略（签名兼容）。
#[cfg(not(windows))]
mod imp {
    use super::*;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Nonce};

    const KEYRING_SERVICE: &str = "xime";
    const KEYRING_USER: &str = "plugin-cipher";

    /// 从 Secret Service 加载（或首次生成并写入）AES-256 主密钥。
    /// keyring 不可用（无 org.freedesktop.se 等）返回 None → 调用方降级明文。
    fn load_or_create_key() -> Option<[u8; KEY_LEN]> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
        match entry.get_password() {
            Ok(stored) => {
                let blob = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    stored.trim(),
                )
                .ok()?;
                if blob.len() != KEY_LEN {
                    return None;
                }
                let mut key = [0u8; KEY_LEN];
                key.copy_from_slice(&blob);
                Some(key)
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = [0u8; KEY_LEN];
                getrandom::fill(&mut key).ok()?;
                let encoded =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);
                entry.set_password(&encoded).ok()?;
                Some(key)
            }
            Err(_) => None,
        }
    }

    pub fn encrypt(plain: &str) -> Result<String, &'static str> {
        let key = load_or_create_key().ok_or("密钥不可用")?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "密钥长度错误")?;
        let mut iv = [0u8; NONCE_LEN];
        getrandom::fill(&mut iv).map_err(|_| "随机数不可用")?;
        // Nonce::from_slice 在 aes-gcm 0.11 已弃用：改用 copy_from_slice 填充。
        let mut nonce = Nonce::default();
        nonce.copy_from_slice(&iv);
        let ct = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plain.as_bytes(),
                    aad: &[],
                },
            )
            .map_err(|_| "加密失败")?;
        let mut out = iv.to_vec();
        out.extend_from_slice(&ct);
        Ok(format!(
            "{}{}",
            ENC_PREFIX,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, out)
        ))
    }

    pub fn decrypt(stored: &str) -> Option<String> {
        let Some(encoded) = stored.strip_prefix(ENC_PREFIX) else {
            // 无前缀：旧版明文兼容
            return Some(stored.to_string());
        };
        let blob =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
        if blob.len() <= NONCE_LEN {
            return None;
        }
        let (iv, ct) = blob.split_at(NONCE_LEN);
        let key = load_or_create_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
        let mut nonce = Nonce::default();
        nonce.copy_from_slice(iv);
        let plain = cipher
            .decrypt(&nonce, Payload { msg: ct, aad: &[] })
            .ok()?;
        String::from_utf8(plain).ok()
    }
}

/// 由插件配置文件路径推导密钥文件路径：
/// `<数据目录>/plugins/config/<id>.yaml` → `<数据目录>/secret.key`。
pub fn key_path_for_config(config_file: &Path) -> PathBuf {
    config_file
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("secret.key"))
        .unwrap_or_else(|| PathBuf::from("secret.key"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_key(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xime_cipher_{label}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_default();
        dir.join("secret.key")
    }

    #[test]
    fn roundtrip_and_plaintext_passthrough() {
        let key = temp_key("roundtrip");
        let stored = encrypt_with_key_path(&key, "应用密码 secret-123");
        if stored.starts_with(ENC_PREFIX) {
            // 密钥平台原语可用（Windows DPAPI / Linux Secret Service）：
            // 应为 enc: 密文且可解回。
            assert_ne!(stored, "应用密码 secret-123", "密文不等于明文");
            assert_eq!(
                decrypt_with_key_path(&key, &stored),
                Some("应用密码 secret-123".to_string())
            );
        } else {
            // 无 Secret Service 的环境：明文降级（与旧版行为一致）。
            assert_eq!(stored, "应用密码 secret-123");
            assert_eq!(
                decrypt_with_key_path(&key, &stored),
                Some("应用密码 secret-123".to_string())
            );
        }
        // 无前缀 → 旧版明文兼容
        assert_eq!(
            decrypt_with_key_path(&key, "plain-value"),
            Some("plain-value".to_string())
        );
    }

    #[cfg(windows)]
    #[test]
    fn tampered_ciphertext_fails_auth() {
        let key = temp_key("tamper");
        let stored = encrypt_with_key_path(&key, "secret");
        let mut blob = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            stored.strip_prefix(ENC_PREFIX).unwrap_or_default(),
        )
        .unwrap_or_default();
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;
        let tampered = format!(
            "{}{}",
            ENC_PREFIX,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, blob)
        );
        assert_eq!(
            decrypt_with_key_path(&key, &tampered),
            None,
            "GCM 认证失败返回 None"
        );
    }

    #[cfg(windows)]
    #[test]
    fn key_file_is_dpapi_blob_not_raw_key() {
        let key = temp_key("keyfile");
        let _ = encrypt_with_key_path(&key, "secret");
        let content = std::fs::read_to_string(&key).unwrap_or_default();
        assert!(!content.is_empty());
        // DPAPI 密文不应包含原始密钥的可读形式；且文件按用户绑定
        assert_ne!(
            content.trim().len(),
            KEY_LEN,
            "密钥文件存的应是 DPAPI 密文而非裸密钥"
        );
    }
}
