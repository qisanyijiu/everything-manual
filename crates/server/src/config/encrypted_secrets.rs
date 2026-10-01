//! Versioned AEAD envelopes. Master keys are injected or kept in the system Keychain,
//! never beside ciphertext. Reading an existing envelope never creates a master key.
use super::SecretString;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

pub const MASTER_ENV: &str = "EM_SECRETS_MASTER_KEY";
pub const KEYCHAIN_SERVICE: &str = "org.everything-manual.secrets.v1";
pub const KEYCHAIN_ACCOUNT: &str = "master";
const MAX_FILE: u64 = 262144;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretError {
    MasterUnavailable,
    InvalidMaster,
    InvalidEnvelope,
    UnsafeFile,
    WriteFailed,
    LegacyFile,
}
impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MasterUnavailable | Self::InvalidMaster => "密钥未能加密保存，本次配置未保存。请检查服务端的系统凭据库访问权限或主密钥环境注入；当前编辑仍保留，可在恢复后重试保存。",
            Self::InvalidEnvelope => "加密配置无法读取，请检查原主密钥与配置文件。不要删除密文或改回明文来绕过此错误。",
            Self::UnsafeFile => "密钥配置文件不可读、不是受限普通文件或超出大小限制；未读取内容。",
            Self::WriteFailed => "无法写入加密配置，请检查服务端目录权限和可用空间；本次配置未保存，当前编辑仍保留。",
            Self::LegacyFile => "检测到旧明文 API 密钥文件；请使用 encrypt-api-key 生成新密文文件并更新 api_key_file 引用，原文件不会自动删除。",
        })
    }
}
impl std::error::Error for SecretError {}

/// Explicit injection boundary: load is read-only; create must not overwrite a winner.
pub trait MasterKeySource: Send + Sync {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretError>;
    fn create(&self) -> Result<Zeroizing<[u8; 32]>, SecretError>;
}
#[derive(Clone)]
pub struct Secrets {
    source: Arc<dyn MasterKeySource>,
    established: Arc<Mutex<Option<Zeroizing<[u8; 32]>>>>,
}
struct Fixed(Zeroizing<[u8; 32]>);
impl MasterKeySource for Fixed {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretError> {
        Ok(Some(self.0.clone()))
    }
    fn create(&self) -> Result<Zeroizing<[u8; 32]>, SecretError> {
        Ok(self.0.clone())
    }
}
struct Unavailable;
impl MasterKeySource for Unavailable {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretError> {
        Ok(None)
    }
    fn create(&self) -> Result<Zeroizing<[u8; 32]>, SecretError> {
        Err(SecretError::MasterUnavailable)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub format: String,
    pub version: u32,
    pub algorithm: String,
    pub purpose: String,
    pub nonce: String,
    pub ciphertext: String,
}
impl Secrets {
    pub fn with_source(source: Arc<dyn MasterKeySource>) -> Self {
        Self {
            source,
            established: Arc::new(Mutex::new(None)),
        }
    }
    pub fn fixed(key: [u8; 32]) -> Self {
        Self::with_source(Arc::new(Fixed(Zeroizing::new(key))))
    }
    pub fn unavailable() -> Self {
        Self::with_source(Arc::new(Unavailable))
    }
    /// Selection does no Keychain IO. Invalid explicit injection never falls back.
    pub fn from_environment() -> Result<Self, SecretError> {
        match std::env::var(MASTER_ENV) {
            Ok(value) => {
                let value = Zeroizing::new(value);
                let bytes =
                    Zeroizing::new(decode_hex(&value).map_err(|_| SecretError::InvalidMaster)?);
                let key: [u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SecretError::InvalidMaster)?;
                Ok(Self::fixed(key))
            }
            Err(std::env::VarError::NotUnicode(_)) => Err(SecretError::InvalidMaster),
            Err(std::env::VarError::NotPresent) => {
                #[cfg(target_os = "macos")]
                {
                    static DEFAULT: std::sync::OnceLock<Secrets> = std::sync::OnceLock::new();
                    Ok(DEFAULT
                        .get_or_init(|| Self::native(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT))
                        .clone())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Ok(Self::unavailable())
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    pub fn native(service: &str, account: &str) -> Self {
        Self::with_source(Arc::new(NativeKeychain::new(service, account)))
    }
    fn master(&self, allow_create: bool) -> Result<Zeroizing<[u8; 32]>, SecretError> {
        let mut established = self
            .established
            .lock()
            .map_err(|_| SecretError::MasterUnavailable)?;
        let current = match self.source.load()? {
            Some(key) => key,
            None if allow_create && established.is_none() => self.source.create()?,
            None => return Err(SecretError::MasterUnavailable),
        };
        if established
            .as_ref()
            .is_some_and(|known| **known != *current)
        {
            return Err(SecretError::InvalidMaster);
        }
        *established = Some(current.clone());
        Ok(current)
    }
    pub fn ensure_existing_key(&self) -> Result<(), SecretError> {
        self.master(false).map(|_| ())
    }
    pub fn encrypt(&self, secret: &SecretString, purpose: &str) -> Result<Envelope, SecretError> {
        let master = self.master(true)?;
        let key = LessSafeKey::new(
            UnboundKey::new(&AES_256_GCM, master.as_ref())
                .map_err(|_| SecretError::InvalidMaster)?,
        );
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|_| SecretError::MasterUnavailable)?;
        let mut bytes = Zeroizing::new(secret.expose().as_bytes().to_vec());
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad(purpose)),
            &mut *bytes,
        )
        .map_err(|_| SecretError::InvalidEnvelope)?;
        Ok(Envelope {
            format: "everything-manual-secret".into(),
            version: 1,
            algorithm: "AES-256-GCM".into(),
            purpose: purpose.into(),
            nonce: encode_hex(&nonce),
            ciphertext: encode_hex(&bytes),
        })
    }
    pub fn decrypt(&self, envelope: &Envelope, purpose: &str) -> Result<SecretString, SecretError> {
        if envelope.format != "everything-manual-secret"
            || envelope.version != 1
            || envelope.algorithm != "AES-256-GCM"
            || envelope.purpose != purpose
        {
            return Err(SecretError::InvalidEnvelope);
        }
        if envelope.nonce.len() != 24 || envelope.ciphertext.len() > 32800 {
            return Err(SecretError::InvalidEnvelope);
        }
        let nonce: [u8; 12] = decode_hex(&envelope.nonce)?
            .as_slice()
            .try_into()
            .map_err(|_| SecretError::InvalidEnvelope)?;
        let mut bytes = Zeroizing::new(decode_hex(&envelope.ciphertext)?);
        let master = self.master(false)?;
        let key = LessSafeKey::new(
            UnboundKey::new(&AES_256_GCM, master.as_ref())
                .map_err(|_| SecretError::InvalidMaster)?,
        );
        let plaintext = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(purpose)),
                &mut bytes,
            )
            .map_err(|_| SecretError::InvalidEnvelope)?;
        let value = std::str::from_utf8(plaintext).map_err(|_| SecretError::InvalidEnvelope)?;
        if !valid_secret(value) {
            return Err(SecretError::InvalidEnvelope);
        }
        Ok(SecretString::new(value))
    }
    pub fn read_api_key(&self, path: &Path, provider: &str) -> Result<SecretString, SecretError> {
        let bytes = read_private_file(path)?;
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| {
            if bytes.first() == Some(&b'{') {
                SecretError::InvalidEnvelope
            } else {
                SecretError::LegacyFile
            }
        })?;
        self.decrypt(&envelope, &format!("api-key-file:{provider}"))
    }
    pub fn encrypt_api_key_file(
        &self,
        input: &Path,
        output: &Path,
        provider: &str,
    ) -> Result<(), SecretError> {
        // Verify both paths before creating any master; never truncate or overwrite output.
        if fs::symlink_metadata(output).is_ok() {
            return Err(SecretError::WriteFailed);
        }
        let bytes = read_private_file(input)?;
        let value = std::str::from_utf8(&bytes)
            .map_err(|_| SecretError::UnsafeFile)?
            .trim_end_matches(['\r', '\n']);
        if !valid_secret(value) {
            return Err(SecretError::UnsafeFile);
        }
        let secret = SecretString::new(value);
        let purpose = format!("api-key-file:{provider}");
        let envelope = self.encrypt(&secret, &purpose)?;
        if self.decrypt(&envelope, &purpose)? != secret {
            return Err(SecretError::InvalidEnvelope);
        }
        write_private_file(
            output,
            &serde_json::to_vec(&envelope).map_err(|_| SecretError::WriteFailed)?,
            false,
        )
    }
}
fn aad(purpose: &str) -> String {
    format!("everything-manual-secret\0v1\0AES-256-GCM\0{purpose}")
}
fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 4096
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode_hex(value: &str) -> Result<Vec<u8>, SecretError> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err(SecretError::InvalidEnvelope);
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |b: u8| (b as char).to_digit(16).ok_or(SecretError::InvalidEnvelope);
            Ok(((digit(pair[0])? << 4) | digit(pair[1])?) as u8)
        })
        .collect()
}
pub fn read_private_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| SecretError::UnsafeFile)?;
    check_metadata(&metadata)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| SecretError::UnsafeFile)?;
    check_metadata(&file.metadata().map_err(|_| SecretError::UnsafeFile)?)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SecretError::UnsafeFile)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(SecretError::UnsafeFile);
    }
    Ok(bytes)
}
fn check_metadata(metadata: &fs::Metadata) -> Result<(), SecretError> {
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(SecretError::UnsafeFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SecretError::UnsafeFile);
        }
    }
    Ok(())
}
/// Replace only a safe existing file, or atomically create a new path (hard-link, no clobber).
pub fn write_private_file(path: &Path, bytes: &[u8], replace: bool) -> Result<(), SecretError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !replace {
                return Err(SecretError::WriteFailed);
            }
            check_metadata(&metadata)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SecretError::WriteFailed),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = fs::File::open(parent).map_err(|_| SecretError::WriteFailed)?;
    sync_directory(&directory, path, false).map_err(|_| SecretError::WriteFailed)?;
    let temporary = path.with_extension(format!("{}.tmp", manual_core::ids::new_id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| SecretError::WriteFailed)?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| SecretError::WriteFailed)?;
        if replace {
            fs::rename(&temporary, path).map_err(|_| SecretError::WriteFailed)?;
        } else {
            fs::hard_link(&temporary, path).map_err(|_| SecretError::WriteFailed)?;
        }
        // Rename/link is the commit point: do not report an uncommitted save after publication.
        if sync_directory(&directory, path, true).is_err() {
            tracing::warn!("加密配置已提交，但目录同步失败；请检查存储设备的持久化能力");
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn sync_directory(directory: &fs::File, _path: &Path, _after_commit: bool) -> std::io::Result<()> {
    #[cfg(feature = "job-failpoints")]
    if test_faults::should_fail(_path, _after_commit) {
        return Err(std::io::Error::other("isolated directory sync fault"));
    }
    directory.sync_all()
}

/// Test-only path-scoped injection. No environment toggle or production management endpoint.
#[cfg(feature = "job-failpoints")]
pub mod test_faults {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Mutex, OnceLock},
    };
    fn faults() -> &'static Mutex<HashMap<PathBuf, bool>> {
        static FAULTS: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
        FAULTS.get_or_init(|| Mutex::new(HashMap::new()))
    }
    pub struct DirectorySyncFault(PathBuf);
    impl Drop for DirectorySyncFault {
        fn drop(&mut self) {
            if let Ok(mut faults) = faults().lock() {
                faults.remove(&self.0);
            }
        }
    }
    /// false=preflight failure, true=post-rename/link failure. Guard removes only its path.
    pub fn fail_directory_sync(path: &Path, after_commit: bool) -> DirectorySyncFault {
        faults()
            .lock()
            .expect("test fault lock")
            .insert(path.to_path_buf(), after_commit);
        DirectorySyncFault(path.to_path_buf())
    }
    pub(super) fn should_fail(path: &Path, after_commit: bool) -> bool {
        faults()
            .lock()
            .is_ok_and(|faults| faults.get(path) == Some(&after_commit))
    }
}

#[cfg(target_os = "macos")]
pub struct NativeKeychain {
    service: String,
    account: String,
}
#[cfg(target_os = "macos")]
impl NativeKeychain {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }
    /// Only deletes this explicitly named credential; for isolated fixture cleanup.
    pub fn delete(&self) -> Result<(), SecretError> {
        let keychain = security_framework::os::macos::keychain::SecKeychain::default()
            .map_err(|_| SecretError::MasterUnavailable)?;
        match keychain.find_generic_password(&self.service, &self.account) {
            Ok((password, item)) => {
                drop(password);
                item.delete();
                Ok(())
            }
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(SecretError::MasterUnavailable),
        }
    }
}
#[cfg(target_os = "macos")]
impl MasterKeySource for NativeKeychain {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretError> {
        let keychain = security_framework::os::macos::keychain::SecKeychain::default()
            .map_err(|_| SecretError::MasterUnavailable)?;
        match keychain.find_generic_password(&self.service, &self.account) {
            Ok((password, _)) => Ok(Some(Zeroizing::new(
                password
                    .as_ref()
                    .try_into()
                    .map_err(|_| SecretError::InvalidMaster)?,
            ))),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(SecretError::MasterUnavailable),
        }
    }
    fn create(&self) -> Result<Zeroizing<[u8; 32]>, SecretError> {
        let mut generated = Zeroizing::new([0u8; 32]);
        getrandom::fill(generated.as_mut()).map_err(|_| SecretError::MasterUnavailable)?;
        let keychain = security_framework::os::macos::keychain::SecKeychain::default()
            .map_err(|_| SecretError::MasterUnavailable)?;
        // SecKeychainAddGenericPassword is create-only; classic default keychain items do not sync to iCloud.
        match keychain.add_generic_password(&self.service, &self.account, generated.as_ref()) {
            Ok(_) => Ok(generated),
            Err(error) if error.code() == -25299 => {
                self.load()?.ok_or(SecretError::MasterUnavailable)
            }
            Err(_) => Err(SecretError::MasterUnavailable),
        }
    }
}
