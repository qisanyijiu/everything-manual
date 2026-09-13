//! 会话 token 与 CSRF token 的生成、哈希与比较（REQ-002、contracts.md §2）。
//!
//! 设计（为什么这样派生）：
//! - **会话 token**：32 字节（256 bit）OS 熵，hex 编码（64 字符）。明文只写入
//!   `Set-Cookie`，**落库只存 SHA-256 哈希**（`sessions.session_token_hash`），
//!   与"明文只返回 cookie，不落库/日志"一致；
//! - **CSRF token 由会话 token 派生**：`csrf = SHA-256(session_token || ":csrf-v1")`。
//!   这样才能在 `GET /auth/session`（只带 cookie、库里只有哈希）时把同一个
//!   CSRF token 交还给前端，而库里仍只保存 `SHA-256(csrf)`（`sessions.csrf_hash`）。
//!   跨站攻击者拿不到 HttpOnly cookie 的明文，也就推不出 CSRF token；
//!   double-submit 校验即比较 `SHA-256(收到的 X-CSRF-Token)` 与库中 `csrf_hash`；
//! - 行内版本后缀 `:csrf-v1` 便于未来更换派生规则时旧会话自然失效。

use sha2::{Digest, Sha256};

/// 会话 token 的随机字节数（256 bit）。
pub const SESSION_TOKEN_BYTES: usize = 32;

/// CSRF token 的派生域分隔后缀（更换规则即让旧会话的 CSRF 失效）。
const CSRF_DERIVATION_SUFFIX: &str = ":csrf-v1";

/// 生成会话 token（hex，64 字符）。失败即 OS 熵源不可用——调用方按 500 处理。
pub fn generate_session_token() -> Result<String, String> {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("无法获得安全随机数：{error}"))?;
    Ok(hex_encode(&bytes))
}

/// 库中保存的会话 token 哈希（查找会话时按此值匹配）。
pub fn session_token_hash(session_token: &str) -> String {
    sha256_hex(session_token.as_bytes())
}

/// 由会话 token 派生 CSRF token（返回给前端；跨站无法推导）。
pub fn csrf_token_for(session_token: &str) -> String {
    let mut input = String::with_capacity(session_token.len() + CSRF_DERIVATION_SUFFIX.len());
    input.push_str(session_token);
    input.push_str(CSRF_DERIVATION_SUFFIX);
    sha256_hex(input.as_bytes())
}

/// 库中保存的 CSRF 哈希（校验时按此值匹配）。
pub fn csrf_hash(csrf_token: &str) -> String {
    sha256_hex(csrf_token.as_bytes())
}

/// 常量时间字符串比较（长度不同直接不相等；哈希值是等长 hex，无长度侧信道）。
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tokens_are_unique_hex_of_expected_length() {
        let first = generate_session_token().unwrap();
        let second = generate_session_token().unwrap();
        assert_eq!(first.len(), SESSION_TOKEN_BYTES * 2);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn hashes_are_sha256_hex_and_stable() {
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let hash = session_token_hash(token);
        assert_eq!(hash.len(), 64);
        // 与 `python3 -c "import hashlib; …"` 的独立计算一致（固定算法口径）。
        assert_eq!(
            hash,
            "a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e"
        );
    }

    #[test]
    fn csrf_is_derived_from_session_token_and_hashed_separately() {
        let token = generate_session_token().unwrap();
        let csrf = csrf_token_for(&token);
        assert_ne!(csrf, token);
        assert_eq!(
            csrf,
            csrf_token_for(&token),
            "同一会话的 CSRF 必须可重复派生"
        );
        assert_ne!(csrf_hash(&csrf), csrf);
        // 库里保存的 csrf_hash 就是 SHA-256(csrf token)。
        assert_eq!(csrf_hash(&csrf), sha256_hex(csrf.as_bytes()));
        // 修改一字节会话 token 会得到不同 CSRF。
        let mut other = token.clone();
        other.push('0');
        assert_ne!(csrf_token_for(&other), csrf);
    }

    #[test]
    fn constant_time_equality_behaves_like_equality() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
    }
}
