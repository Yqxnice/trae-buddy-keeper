//! 账户敏感凭证的静态加密（Windows DPAPI）。
//!
//! `accounts` 表中的 `access_token` / `refresh_token` / `cookies` / `device_key`
//! 以 `dpapi:v1:<base64>` 形式落库，密文由 `CryptProtectData` 绑定当前 Windows 用户，
//! 其他用户或设备无法解密。空字符串保持为空，避免破坏调用方普遍的 `is_empty()` 判定。
//!
//! 兼容性：
//! - 无前缀的旧明文按原文读取（`open`），由 `Database` 在启动时就地迁移为密文；
//! - 密文随数据库备份一同复制，**仅同一 Windows 用户可恢复**。跨用户 / 跨设备恢复时
//!   解密会失败，此时凭证按空值处理并提示重新授权登录，绝不把乱码当作 token 发出去。
//! - 非 Windows 平台不提供加密（本项目为 Windows 桌面应用），仅保留明文透传以便编译检查。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// 密文前缀；同时用于区分旧明文与已加密数据。
const SEALED_PREFIX: &str = "dpapi:v1:";

pub fn is_sealed(s: &str) -> bool {
    s.starts_with(SEALED_PREFIX)
}

#[cfg(windows)]
mod platform {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// 取出 DPAPI 输出缓冲区并释放其 LocalAlloc 内存。
    /// # Safety
    /// `out.pbData` 必须是本次 DPAPI 调用分配的有效指针，且只可调用一次。
    unsafe fn take_output(out: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if out.pbData.is_null() || out.cbData == 0 {
            return Vec::new();
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        bytes
    }

    pub(super) fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("DPAPI 加密失败: {e}"))?;
        }
        let bytes = unsafe { take_output(&output) };
        if bytes.is_empty() {
            return Err("DPAPI 返回空密文".to_string());
        }
        Ok(bytes)
    }

    pub(super) fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("DPAPI 解密失败（凭证可能属于其他 Windows 用户或设备）: {e}"))?;
        }
        let bytes = unsafe { take_output(&output) };
        if bytes.is_empty() {
            return Err("DPAPI 返回空明文".to_string());
        }
        Ok(bytes)
    }
}

/// 加密待落库的明文。空值原样返回；已加密的值幂等返回。
#[cfg(windows)]
pub fn seal(plain: &str) -> Result<String, String> {
    if plain.is_empty() || is_sealed(plain) {
        return Ok(plain.to_string());
    }
    Ok(format!(
        "{SEALED_PREFIX}{}",
        B64.encode(platform::protect(plain.as_bytes())?)
    ))
}

/// 读取库中存储的值：旧明文透传，密文解密。解密失败返回 Err，由调用方按空凭证处理。
#[cfg(windows)]
pub fn open(stored: &str) -> Result<String, String> {
    if stored.is_empty() || !is_sealed(stored) {
        return Ok(stored.to_string());
    }
    let blob = B64
        .decode(&stored[SEALED_PREFIX.len()..])
        .map_err(|e| format!("凭证密文编码损坏: {e}"))?;
    let plain = platform::unprotect(&blob)?;
    String::from_utf8(plain).map_err(|e| format!("解密结果不是合法 UTF-8: {e}"))
}

/// 非 Windows 平台仅用于开发期编译检查，不提供加密能力。
#[cfg(not(windows))]
pub fn seal(plain: &str) -> Result<String, String> {
    eprintln!("[crypto] 非 Windows 平台不提供 DPAPI，凭证以明文存储（仅供开发）");
    Ok(plain.to_string())
}

#[cfg(not(windows))]
pub fn open(stored: &str) -> Result<String, String> {
    if is_sealed(stored) {
        return Err("当前平台无法解密 DPAPI 凭证".to_string());
    }
    Ok(stored.to_string())
}

/// 解出 DPAPI 保护的二进制数据：客户端 Electron `Local State` 里的 AES 密钥就是这种形态。
/// 只用于本机只读核验登录身份，不落库、不写日志。
#[cfg(windows)]
pub fn unprotect_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    platform::unprotect(data)
}

#[cfg(not(windows))]
pub fn unprotect_bytes(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("当前平台不支持 DPAPI 解密".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_value_stays_empty() {
        // 空值必须保持空，否则 `access_token.is_empty()` / `cookies.trim().is_empty()`
        // 等凭证判定会全部失效。
        assert_eq!(seal("").unwrap(), "");
        assert_eq!(open("").unwrap(), "");
    }

    #[test]
    fn legacy_plaintext_passthrough() {
        assert!(!is_sealed("plain-token"));
        assert_eq!(open("plain-token").unwrap(), "plain-token");
    }

    #[test]
    fn roundtrip_keeps_value_intact() {
        let value = "eyJhbGciOi-中文-123";
        let sealed = seal(value).unwrap();
        assert_eq!(open(&sealed).unwrap(), value);
    }

    #[cfg(windows)]
    #[test]
    fn windows_seal_is_marked_and_idempotent() {
        let sealed = seal("secret").unwrap();
        assert!(is_sealed(&sealed));
        assert_ne!(sealed, "secret");
        assert_eq!(seal(&sealed).unwrap(), sealed);
    }

    #[cfg(windows)]
    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut bad = seal("secret").unwrap();
        bad.push('A');
        match open(&bad) {
            Err(_) => {}
            Ok(v) => assert_ne!(v, "secret", "损坏的密文绝不能还原出原值"),
        }
    }
}
