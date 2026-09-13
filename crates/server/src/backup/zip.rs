//! 最小 ZIP 写入器（**STORE 无压缩**；T20 导出自包含包用）。
//!
//! 为什么自己写而不是引入压缩 crate：
//! - 导出包不做压缩（原件 PDF/GLB 本身是二进制资产，压缩收益有限），STORE 模式的
//!   ZIP 格式只依赖 CRC32 + 固定结构，实现完全可控（约 200 行），**不新增依赖**；
//! - 字节确定性：所有条目使用固定 DOS 时间戳（1980-01-01），同一 release 的导出包
//!   字节一致（测试可断言"两次导出相同"）；
//! - 条目名只由服务端生成（角色 + sha256），但写入前仍做转义检查（无 `..`、无绝对
//!   路径、无反斜杠）——导出包将来若开放导入，zip-slip 验收仍须另立（contracts §7
//!   明确"当前不顺手开放 ZIP 导入接口"）。
//!
//! 格式限制（超出即报错，不写坏包）：条目数 ≤ 65535、单条目与总量 < 4 GiB
//! （MVP 资产预算：GLB 150 MiB / PDF 50 MiB / manifest 8 MiB，远小于上限）。
//!
//! 不使用 data descriptor（flag bit 3）：每个条目先扫描一遍得到 CRC 与大小，
//! 再写 local header 与数据——保证最保守的解压工具（含旧版 `unzip`）都能读。
//!
//! 解压验证入口（QA 与手工演练）：`unzip -t <包>`、`python3 -m zipfile -t <包>`、
//! 或 `open`/双击由系统解压；自动化测试用 [`parse_stored_zip`] 做结构 + CRC 校验。

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::error::BackupError;
use super::files::{CHUNK_BYTES, FileFingerprint};

/// 固定的 DOS 时间戳：1980-01-01 00:00（ZIP 的 DOS 纪元起点）。
/// 固定值让同一内容的导出包字节确定。
const DOS_DATE: u16 = 0x0021; // (1980-1980) << 9 | 1 << 5 | 1
const DOS_TIME: u16 = 0;

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const VERSION_NEEDED: u16 = 20; // 2.0（store 方法的最小版本）
const METHOD_STORE: u16 = 0;

/// ZIP 单条目 / 总量的格式上限（u32 字段）。
pub const ZIP_MAX_ENTRY_BYTES: u64 = 0xFFFF_FFFF;
pub const ZIP_MAX_ENTRIES: usize = u16::MAX as usize;

/// CRC32（IEEE 802.3，反射式多项式 0xEDB88320；ZIP 使用同一标准）。
#[derive(Debug, Clone)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        let mut state = self.state;
        for byte in bytes {
            let index = ((state ^ u32::from(*byte)) & 0xFF) as usize;
            state = (state >> 8) ^ CRC_TABLE[index];
        }
        self.state = state;
    }

    pub fn finalize(&self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

/// CRC32 查表（编译期生成，无运行时初始化开销）。
const CRC_TABLE: [u32; 256] = build_crc_table();

const fn build_crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// 一次性 CRC32（小数据用；大文件用 [`Crc32`] 流式）。
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finalize()
}

/// 条目数据来源。
pub enum ZipEntrySource<'a> {
    /// 内存中的字节（manifest 等小文件）。
    Bytes(&'a [u8]),
    /// 磁盘文件 + 已扫描到的指纹（[`FileFingerprint`]；由调用方先校验内容）。
    File {
        path: &'a Path,
        fingerprint: &'a FileFingerprint,
    },
}

/// 一个待写入的 ZIP 条目。
pub struct ZipEntry<'a> {
    /// POSIX 相对路径（`/` 分隔；不含前导 `/`、`..` 或反斜杠）。
    pub name: String,
    pub source: ZipEntrySource<'a>,
}

struct CentralRecord {
    name: String,
    crc32: u32,
    size: u64,
    offset: u64,
}

/// 把条目写入 `writer`（调用方负责最终落盘/流式响应）。
pub async fn write_stored_zip<W>(
    mut writer: W,
    entries: &[ZipEntry<'_>],
) -> Result<u64, BackupError>
where
    W: AsyncWriteExt + Unpin,
{
    if entries.len() > ZIP_MAX_ENTRIES {
        return Err(BackupError::io(format!(
            "导出包条目数超过 ZIP 格式上限（{} > {ZIP_MAX_ENTRIES}）",
            entries.len()
        )));
    }
    let mut records: Vec<CentralRecord> = Vec::with_capacity(entries.len());
    let mut offset: u64 = 0;

    for entry in entries {
        validate_entry_name(&entry.name)?;
        let name_len = u16::try_from(entry.name.len())
            .map_err(|_| BackupError::io(format!("导出包条目名过长：{}", entry.name)))?;

        let (crc32, size) = match &entry.source {
            ZipEntrySource::Bytes(bytes) => {
                (crc32(bytes), u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            }
            ZipEntrySource::File { fingerprint, .. } => (fingerprint.crc32, fingerprint.size),
        };
        if size >= ZIP_MAX_ENTRY_BYTES {
            return Err(BackupError::io(format!(
                "条目超过 ZIP 格式的单文件上限（{} 字节）：{}",
                ZIP_MAX_ENTRY_BYTES, entry.name
            )));
        }

        // 本条目的 local header 偏移（中央目录要回填）。
        let record_offset = offset;
        let mut header = Vec::with_capacity(30 + entry.name.len());
        push_u32(&mut header, LOCAL_HEADER_SIGNATURE);
        push_u16(&mut header, VERSION_NEEDED);
        push_u16(&mut header, 0); // flags：无 data descriptor、无加密
        push_u16(&mut header, METHOD_STORE);
        push_u16(&mut header, DOS_TIME);
        push_u16(&mut header, DOS_DATE);
        push_u32(&mut header, crc32);
        push_u32(&mut header, size as u32);
        push_u32(&mut header, size as u32);
        push_u16(&mut header, name_len);
        push_u16(&mut header, 0); // extra length
        header.extend_from_slice(entry.name.as_bytes());
        writer.write_all(&header).await.map_err(zip_io)?;
        offset += header.len() as u64;

        match &entry.source {
            ZipEntrySource::Bytes(bytes) => {
                writer.write_all(bytes).await.map_err(zip_io)?;
                offset += bytes.len() as u64;
            }
            ZipEntrySource::File { path, fingerprint } => {
                let mut file = tokio::fs::File::open(path)
                    .await
                    .map_err(|error| zip_io_file(error, path))?;
                let mut buffer = vec![0_u8; CHUNK_BYTES];
                let mut written: u64 = 0;
                loop {
                    let read = file
                        .read(&mut buffer)
                        .await
                        .map_err(|error| zip_io_file(error, path))?;
                    if read == 0 {
                        break;
                    }
                    writer.write_all(&buffer[..read]).await.map_err(zip_io)?;
                    written += read as u64;
                }
                if written != fingerprint.size {
                    return Err(BackupError::integrity(
                        "export_asset_size_changed",
                        format!(
                            "读取资产时大小发生变化（{} 字节 → {} 字节）：{}",
                            fingerprint.size, written, entry.name
                        ),
                    ));
                }
                offset += written;
            }
        }

        records.push(CentralRecord {
            name: entry.name.clone(),
            crc32,
            size,
            offset: record_offset,
        });
    }

    // 中央目录。
    let central_start = offset;
    let mut central = Vec::new();
    for record in &records {
        push_u32(&mut central, CENTRAL_HEADER_SIGNATURE);
        push_u16(&mut central, VERSION_NEEDED); // version made by
        push_u16(&mut central, VERSION_NEEDED); // version needed
        push_u16(&mut central, 0); // flags
        push_u16(&mut central, METHOD_STORE);
        push_u16(&mut central, DOS_TIME);
        push_u16(&mut central, DOS_DATE);
        push_u32(&mut central, record.crc32);
        push_u32(&mut central, record.size as u32);
        push_u32(&mut central, record.size as u32);
        push_u16(&mut central, record.name.len() as u16);
        push_u16(&mut central, 0); // extra
        push_u16(&mut central, 0); // comment
        push_u16(&mut central, 0); // disk number start
        push_u16(&mut central, 0); // internal attributes
        push_u32(&mut central, 0); // external attributes
        push_u32(&mut central, record.offset as u32);
        central.extend_from_slice(record.name.as_bytes());
    }
    writer.write_all(&central).await.map_err(zip_io)?;
    offset += central.len() as u64;

    let mut eocd = Vec::with_capacity(22);
    push_u32(&mut eocd, EOCD_SIGNATURE);
    push_u16(&mut eocd, 0); // this disk
    push_u16(&mut eocd, 0); // disk with central directory
    push_u16(&mut eocd, records.len() as u16);
    push_u16(&mut eocd, records.len() as u16);
    push_u32(&mut eocd, central.len() as u32);
    push_u32(&mut eocd, central_start as u32);
    push_u16(&mut eocd, 0); // comment length
    writer.write_all(&eocd).await.map_err(zip_io)?;
    offset += eocd.len() as u64;

    writer.flush().await.map_err(zip_io)?;
    Ok(offset)
}

fn zip_io(error: std::io::Error) -> BackupError {
    BackupError::io(format!("写入导出包失败：{error}"))
}

fn zip_io_file(error: std::io::Error, path: &Path) -> BackupError {
    BackupError::io(format!("读取导出资产失败（{}）：{error}", path.display()))
}

/// 条目名安全校验（与备份 manifest 的相对路径同一口径 + ZIP 特有约定）。
fn validate_entry_name(name: &str) -> Result<(), BackupError> {
    super::manifest::validate_relative_path(name).map_err(|error| match error {
        BackupError::Integrity { message, .. } => {
            BackupError::integrity("export_path_unsafe", message)
        }
        other => other,
    })?;
    if name.ends_with('/') {
        return Err(BackupError::integrity(
            "export_path_unsafe",
            format!("导出包条目不使用目录项：{name}"),
        ));
    }
    Ok(())
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

/// 解析一个 STORE 模式的 ZIP（独立的读取实现，用于测试与自检）。
///
/// 只支持本写入器产出的形态：无加密、无注释、无 zip64；返回 (条目名, 解出的字节)。
/// 校验每个条目的 CRC32 与中央目录一致性——`unzip -t` 之外的自动化等价物。
pub fn parse_stored_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, BackupError> {
    let invalid = |message: &str| BackupError::integrity("zip_invalid", message.to_owned());

    if bytes.len() < 22 {
        return Err(invalid("ZIP 短于 EOCD"));
    }
    let eocd_offset = bytes
        .windows(4)
        .rposition(|window| window == EOCD_SIGNATURE.to_le_bytes())
        .ok_or_else(|| invalid("找不到 EOCD 签名"))?;
    let read_u16 =
        |offset: usize| -> u16 { u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) };
    let read_u32 = |offset: usize| -> u32 {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let entries = read_u16(eocd_offset + 10) as usize;
    let central_size = read_u32(eocd_offset + 12) as usize;
    let central_offset = read_u32(eocd_offset + 16) as usize;
    if central_offset + central_size != eocd_offset {
        return Err(invalid("中央目录与 EOCD 不连续"));
    }

    let mut result = Vec::with_capacity(entries);
    let mut cursor = central_offset;
    for _ in 0..entries {
        if read_u32(cursor) != CENTRAL_HEADER_SIGNATURE {
            return Err(invalid("中央目录条目签名错误"));
        }
        let crc = read_u32(cursor + 16);
        let size = read_u32(cursor + 24) as usize;
        let name_len = read_u16(cursor + 28) as usize;
        let extra_len = read_u16(cursor + 30) as usize;
        let comment_len = read_u16(cursor + 32) as usize;
        let local_offset = read_u32(cursor + 42) as usize;
        let name = String::from_utf8(bytes[cursor + 46..cursor + 46 + name_len].to_vec())
            .map_err(|_| invalid("条目名不是 UTF-8"))?;

        if read_u32(local_offset) != LOCAL_HEADER_SIGNATURE {
            return Err(invalid("local header 签名错误"));
        }
        let local_name_len = read_u16(local_offset + 26) as usize;
        let local_extra_len = read_u16(local_offset + 28) as usize;
        let data_start = local_offset + 30 + local_name_len + local_extra_len;
        let data = bytes
            .get(data_start..data_start + size)
            .ok_or_else(|| invalid("条目数据越界"))?
            .to_vec();
        if crc32(&data) != crc {
            return Err(invalid("条目 CRC32 不符"));
        }
        result.push((name, data));
        cursor += 46 + name_len + extra_len + comment_len;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn crc32_matches_standard_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
        // 流式与一次性一致。
        let mut streaming = Crc32::new();
        streaming.update(b"12345");
        streaming.update(b"6789");
        assert_eq!(streaming.finalize(), 0xCBF4_3926);
    }

    #[tokio::test]
    async fn written_zip_round_trips_through_independent_parser() {
        let payload = vec![7_u8; 300_000];
        let entries = vec![
            ZipEntry {
                name: "manifest.json".to_owned(),
                source: ZipEntrySource::Bytes(br#"{"schemaVersion":"x"}"#),
            },
            ZipEntry {
                name: "assets/document/abc.pdf".to_owned(),
                source: ZipEntrySource::Bytes(&payload),
            },
        ];
        let mut buffer: Vec<u8> = Vec::new();
        let written = write_stored_zip(&mut buffer, &entries).await.unwrap();
        assert_eq!(written as usize, buffer.len());

        let parsed = parse_stored_zip(&buffer).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "manifest.json");
        assert_eq!(parsed[0].1, br#"{"schemaVersion":"x"}"#);
        assert_eq!(parsed[1].0, "assets/document/abc.pdf");
        assert_eq!(parsed[1].1, payload);

        // 字节确定性：同一输入两次写入结果一致（固定时间戳）。
        let mut second: Vec<u8> = Vec::new();
        write_stored_zip(&mut second, &entries).await.unwrap();
        assert_eq!(buffer, second);
    }

    #[tokio::test]
    async fn unsafe_entry_names_are_rejected() {
        for name in ["../escape", "/absolute", "blobs\\win"] {
            let entries = vec![ZipEntry {
                name: name.to_owned(),
                source: ZipEntrySource::Bytes(b"x"),
            }];
            let mut buffer: Vec<u8> = Vec::new();
            let error = write_stored_zip(&mut buffer, &entries).await.unwrap_err();
            assert!(error.to_string().contains("路径不安全"), "{name}: {error}");
            assert!(buffer.is_empty(), "拒绝后不得写出任何字节");
        }
    }

    #[tokio::test]
    async fn parser_rejects_corrupted_payload() {
        let mut buffer: Vec<u8> = Vec::new();
        let entries = vec![ZipEntry {
            name: "a.txt".to_owned(),
            source: ZipEntrySource::Bytes(b"hello zip"),
        }];
        write_stored_zip(&mut buffer, &entries).await.unwrap();
        let position = buffer
            .windows(9)
            .position(|window| window == b"hello zip")
            .expect("数据在包内");
        buffer[position] ^= 0xFF;
        let error = parse_stored_zip(&buffer).unwrap_err();
        assert_eq!(error.code(), "zip_invalid");
    }
}
