//! 上传内容的类型与结构校验（magic 优先，**不信任** Content-Type 与后缀）。
//!
//! 规则（contracts.md §7、PRD §5.3；QA 按此复核）：
//! - 内容类型按文件头 magic 判定，客户端声明的 `Content-Type` 与扩展名只作参考；
//! - `document` 必须是可解析 PDF（`%PDF-` 头 + `%%EOF` + `startxref` + `/Root`，
//!   页数上限见 [`crate::assets::pdf`]）；页数超限 422（`details.pageCount`）；
//! - `photo` / `pageImage` 必须是 JPEG 或 PNG，且尺寸与解码像素在预算内
//!   （像素炸弹 422；结构损坏/截断 422）；
//! - `pageText` 必须是合法 UTF-8 文本（页文字由浏览器 PDF.js 提取后上传）；
//! - 内容类型与用途不符 → 415。
//!
//! 边界：这是**结构级**校验（容器、chunk CRC、marker 链、页树），不是完整像素解码；
//! 真正的像素级渲染/解码在浏览器（T09/T18）完成。这样既拦住伪造类型与像素炸弹，
//! 又不必在服务端引入图片解码库。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use manual_core::domain::AssetPurpose;

use crate::config::Limits;

use super::error::AssetError;
use super::pdf::{self, PdfStructure};

/// 单边像素上限（T06 默认；PRD 未给定具体数值，见 implementation §T06 已知限制）。
pub const MAX_IMAGE_DIMENSION: u32 = 20_000;
/// 解码像素总数上限（80 MP：覆盖 48 MP 手机照片与 8K，拦住"小文件、巨尺寸"的像素炸弹）。
pub const MAX_IMAGE_PIXELS: u64 = 80_000_000;
/// `pageText` 体积上限（PRD §5.3："JSON 请求 ≤1 MiB，页文字另设更高但有限的上限"；
/// 本卡取 2 MiB 常量，单页文字远低于此值；如需可配置留给后续决策）。
pub const PAGE_TEXT_MAX_BYTES: u64 = 2 * 1_048_576;
/// multipart 边界/部件头/文件名等非文件字节的预留（路由级请求体上限 = 单用途上限 + 本值）。
pub const MULTIPART_OVERHEAD_BYTES: u64 = 1_048_576;

/// 某用途允许的单文件字节上限。
///
/// `Model` 不在本路由的取值集合内（T13 的模型下载另有通道），返回 0 表示不接受；
/// `ReleaseManifest` 由发布事务内部生成（T19），同样不接受上传。
pub fn purpose_limit(limits: &Limits, purpose: AssetPurpose) -> u64 {
    match purpose {
        AssetPurpose::Document => limits.max_pdf_bytes,
        AssetPurpose::Photo | AssetPurpose::PageImage => limits.max_photo_bytes,
        AssetPurpose::PageText => PAGE_TEXT_MAX_BYTES,
        AssetPurpose::Model | AssetPurpose::ReleaseManifest => 0,
    }
}

/// 路由级请求体上限：**解析前**就按最大允许用途 + multipart 开销拒绝，而不是先收完再判。
pub fn max_upload_request_bytes(limits: &Limits) -> u64 {
    let largest = [
        limits.max_pdf_bytes,
        limits.max_photo_bytes,
        PAGE_TEXT_MAX_BYTES,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    largest.saturating_add(MULTIPART_OVERHEAD_BYTES)
}

/// 判定出的内容类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentKind {
    Pdf(PdfStructure),
    Png { width: u32, height: u32 },
    Jpeg { width: u32, height: u32 },
    Text,
}

/// 校验通过的内容描述（写库时用其 `mime`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub mime: &'static str,
    pub kind: ContentKind,
}

/// 按用途校验 `path`（已落 tmp、已 fsync 的完整内容）。
pub fn validate(
    path: &Path,
    purpose: AssetPurpose,
    limits: &Limits,
) -> Result<Detected, AssetError> {
    let head = read_head(path, 16)?;
    match purpose {
        AssetPurpose::Document => validate_pdf(path, limits, &head),
        AssetPurpose::Photo | AssetPurpose::PageImage => validate_image(path, &head),
        AssetPurpose::PageText => {
            validate_utf8(path)?;
            Ok(Detected {
                mime: "text/plain; charset=utf-8",
                kind: ContentKind::Text,
            })
        }
        // handler 在进入本函数前已拒绝 model 与 release_manifest（本路由不接受这些 purpose）。
        AssetPurpose::Model | AssetPurpose::ReleaseManifest => Err(AssetError::invalid_content(
            "purpose 非法：本接口只接受 document / photo / pageImage / pageText",
        )),
    }
}

fn validate_pdf(path: &Path, limits: &Limits, head: &[u8]) -> Result<Detected, AssetError> {
    if !head.starts_with(b"%PDF-") {
        return Err(AssetError::unsupported_type(
            "文件内容不是 PDF（缺少 %PDF- 头）：Content-Type 与扩展名不作为判据",
        ));
    }
    let structure = pdf::probe(path)
        .map_err(|reason| AssetError::invalid_content(format!("PDF 结构校验失败：{reason}")))?;
    match &structure {
        PdfStructure::Parsed { page_count } => {
            let page_count = *page_count;
            if page_count == 0 {
                return Err(AssetError::invalid_content("PDF 页数为 0，无法用于准备"));
            }
            if page_count > limits.max_pdf_pages {
                return Err(AssetError::invalid_content_with_details(
                    format!(
                        "PDF 页数超过上限：{} 页 > {} 页（PRD §5.3）",
                        page_count, limits.max_pdf_pages
                    ),
                    serde_json::json!({
                        "reason": "pdfPageLimit",
                        "pageCount": page_count,
                        "maxPages": limits.max_pdf_pages,
                    }),
                ));
            }
        }
        PdfStructure::Encrypted => {
            // REQ-012：加密 PDF 的拒绝发生在准备阶段（T09）；此处接受但页数不可判定。
            tracing::warn!(
                stage = "pdf_probe",
                "PDF 带 /Encrypt：上传接受，页数上限与解密拒绝由 T09 准备阶段处理"
            );
        }
        PdfStructure::Unparsed { reason } => {
            tracing::warn!(
                stage = "pdf_probe",
                reason = %reason,
                "PDF 页树未在本卡探针范围内定位：放行上传，页数上限由 T09 准备阶段权威判定"
            );
        }
    }
    Ok(Detected {
        mime: "application/pdf",
        kind: ContentKind::Pdf(structure),
    })
}

fn validate_image(path: &Path, head: &[u8]) -> Result<Detected, AssetError> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        let (width, height) = png_dimensions_and_layout(path)?;
        check_pixel_budget(width, height)?;
        return Ok(Detected {
            mime: "image/png",
            kind: ContentKind::Png { width, height },
        });
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let (width, height) = jpeg_dimensions_and_layout(path)?;
        check_pixel_budget(width, height)?;
        return Ok(Detected {
            mime: "image/jpeg",
            kind: ContentKind::Jpeg { width, height },
        });
    }
    Err(AssetError::unsupported_type(
        "照片仅接受 JPEG／PNG（按文件头判定，不以 Content-Type 或扩展名为准；\
         HEIC/WebP 需先自行转换）",
    ))
}

fn check_pixel_budget(width: u32, height: u32) -> Result<(), AssetError> {
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION || pixels > MAX_IMAGE_PIXELS {
        return Err(AssetError::invalid_content_with_details(
            format!(
                "图片尺寸超出解码预算：{width}×{height}（单边上限 {MAX_IMAGE_DIMENSION}px，\
                 像素上限 {MAX_IMAGE_PIXELS}）"
            ),
            serde_json::json!({
                "reason": "imagePixels",
                "width": width,
                "height": height,
                "maxDimension": MAX_IMAGE_DIMENSION,
                "maxPixels": MAX_IMAGE_PIXELS,
            }),
        ));
    }
    Ok(())
}

/// PNG：签名 + chunk 布局 + 每个 chunk 的 CRC + IHDR 尺寸 + 必须以 IEND 结束且无尾随字节。
///
/// 逐块流式读取（内存占用与 chunk 大小无关），能识别截断与损坏内容（"解码失败"）。
fn png_dimensions_and_layout(path: &Path) -> Result<(u32, u32), AssetError> {
    let mut file = open(path)?;
    let mut header = [0_u8; 8];
    file.read_exact(&mut header)
        .map_err(|error| AssetError::invalid_content(format!("PNG 读取失败：{error}")))?;
    if &header != b"\x89PNG\r\n\x1a\n" {
        return Err(AssetError::invalid_content("PNG 签名不符"));
    }

    let mut dimensions: Option<(u32, u32)> = None;
    let mut saw_idat = false;
    let mut saw_iend = false;
    let mut buffer = vec![0_u8; 64 * 1024];
    while !saw_iend {
        let mut chunk_header = [0_u8; 8];
        match file.read_exact(&mut chunk_header) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(AssetError::invalid_content("PNG 在 IEND 之前被截断"));
            }
            Err(error) => {
                return Err(AssetError::invalid_content(format!(
                    "PNG 读取失败：{error}"
                )));
            }
        }
        let length = u32::from_be_bytes(chunk_header[0..4].try_into().expect("4 字节")) as u64;
        let chunk_type: [u8; 4] = chunk_header[4..8].try_into().expect("4 字节");

        let mut crc = Crc32::new();
        crc.update(&chunk_type);
        let mut remaining = length;
        let mut first_pass: Vec<u8> = Vec::new();
        while remaining > 0 {
            let take = remaining.min(buffer.len() as u64) as usize;
            if file.read_exact(&mut buffer[..take]).is_err() {
                return Err(AssetError::invalid_content("PNG 在 IEND 之前被截断"));
            }
            crc.update(&buffer[..take]);
            if chunk_type == *b"IHDR" && first_pass.len() < 13 {
                let needed = (13 - first_pass.len()).min(take);
                first_pass.extend_from_slice(&buffer[..needed]);
            }
            remaining -= take as u64;
        }
        let mut crc_bytes = [0_u8; 4];
        if file.read_exact(&mut crc_bytes).is_err() {
            return Err(AssetError::invalid_content(
                "PNG chunk CRC 缺失（文件被截断）",
            ));
        }
        let declared = u32::from_be_bytes(crc_bytes);
        if declared != crc.finish() {
            return Err(AssetError::invalid_content(format!(
                "PNG chunk {chunk_type:?} CRC 不符（内容损坏）"
            )));
        }

        match &chunk_type {
            b"IHDR" => {
                if dimensions.is_some() {
                    return Err(AssetError::invalid_content("PNG 出现第二个 IHDR"));
                }
                if first_pass.len() != 13 {
                    return Err(AssetError::invalid_content("PNG IHDR 长度不是 13"));
                }
                let width = u32::from_be_bytes(first_pass[0..4].try_into().expect("4 字节"));
                let height = u32::from_be_bytes(first_pass[4..8].try_into().expect("4 字节"));
                dimensions = Some((width, height));
            }
            b"IDAT" => saw_idat = true,
            b"IEND" => {
                if length != 0 {
                    return Err(AssetError::invalid_content("PNG IEND 数据段应为空"));
                }
                saw_iend = true;
            }
            _ => {}
        }
    }

    let position = file
        .stream_position()
        .map_err(|error| AssetError::invalid_content(format!("PNG 读取位置失败：{error}")))?;
    let total = file
        .metadata()
        .map_err(|error| AssetError::invalid_content(format!("PNG 读取大小失败：{error}")))?
        .len();
    if position != total {
        return Err(AssetError::invalid_content(
            "PNG 在 IEND 之后仍有数据（结构不合法）",
        ));
    }

    let dimensions = dimensions.ok_or_else(|| AssetError::invalid_content("PNG 缺少 IHDR"))?;
    if !saw_idat {
        return Err(AssetError::invalid_content("PNG 缺少 IDAT 图像数据"));
    }
    Ok(dimensions)
}

/// JPEG：SOI…SOF（尺寸）…SOS…EOI 的 marker 链校验。
///
/// 不逐字节解析熵编码数据，但要求 SOF 尺寸存在、SOS 存在、文件以 EOI（`FF D9`）结束，
/// 并校验各段的声明长度（截断/损坏内容因此返回 422）。
fn jpeg_dimensions_and_layout(path: &Path) -> Result<(u32, u32), AssetError> {
    let mut file = open(path)?;
    let total = file
        .metadata()
        .map_err(|error| AssetError::invalid_content(format!("JPEG 读取大小失败：{error}")))?
        .len();
    if total < 4 {
        return Err(AssetError::invalid_content("JPEG 文件过短"));
    }
    let end = read_tail(&mut file, total, 2)?;
    if end != [0xFF, 0xD9] {
        return Err(AssetError::invalid_content(
            "JPEG 缺少 EOI（文件被截断或损坏）",
        ));
    }

    let mut offset = 2_u64;
    let mut dimensions: Option<(u32, u32)> = None;
    let mut saw_start_of_scan = false;
    let mut buffer = vec![0_u8; 64 * 1024];
    while offset + 2 <= total {
        let pair = read_at(&mut file, offset, 2, &mut buffer)?;
        if pair[0] != 0xFF {
            return Err(AssetError::invalid_content(format!(
                "JPEG marker 位置 {offset} 不是 0xFF（结构损坏）"
            )));
        }
        let marker = pair[1];
        offset += 2;
        match marker {
            0xFF => {
                offset -= 1; // 填充字节
                continue;
            }
            0x01 | 0xD0..=0xD7 => continue, // 无长度段
            0xD9 => break,                  // EOI
            0xDA => {
                saw_start_of_scan = true;
                break; // 熵编码数据之后不再逐字节解析，但 EOI 已在上面校验
            }
            _ => {}
        }
        if offset + 2 > total {
            return Err(AssetError::invalid_content(
                "JPEG 段长度字段缺失（文件被截断）",
            ));
        }
        let length_bytes = read_at(&mut file, offset, 2, &mut buffer)?;
        let length = u16::from_be_bytes([length_bytes[0], length_bytes[1]]) as u64;
        if length < 2 || offset + length > total {
            return Err(AssetError::invalid_content(format!(
                "JPEG marker {marker:#04x} 段长度非法：{length}"
            )));
        }
        if matches!(marker, 0xC0..=0xC2) {
            if length < 8 {
                return Err(AssetError::invalid_content("JPEG SOF 段过短"));
            }
            let sof = read_at(&mut file, offset + 2, 6, &mut buffer)?;
            let height = u32::from(u16::from_be_bytes([sof[1], sof[2]]));
            let width = u32::from(u16::from_be_bytes([sof[3], sof[4]]));
            dimensions = Some((width, height));
        }
        offset += length;
    }

    if !saw_start_of_scan {
        return Err(AssetError::invalid_content("JPEG 缺少 SOS（扫描数据）"));
    }
    dimensions.ok_or_else(|| AssetError::invalid_content("JPEG 缺少 SOF（尺寸不可判定）"))
}

/// `pageText`：整体必须是合法 UTF-8（分块校验，最多保留 3 字节尾部）。
fn validate_utf8(path: &Path) -> Result<(), AssetError> {
    let mut file = open(path)?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AssetError::invalid_content(format!("读取文本失败：{error}")))?;
        if read == 0 {
            break;
        }
        let mut window = carry.clone();
        window.extend_from_slice(&buffer[..read]);
        match std::str::from_utf8(&window) {
            Ok(_) => carry.clear(),
            Err(error) => {
                let valid = error.valid_up_to();
                match error.error_len() {
                    // 末尾不完整：留作下一轮的 carry（最多 3 字节）。
                    None => carry = window[valid..].to_vec(),
                    Some(_) => {
                        return Err(AssetError::invalid_content("pageText 不是合法 UTF-8 文本"));
                    }
                }
            }
        }
        if carry.len() > 3 {
            return Err(AssetError::invalid_content("pageText 不是合法 UTF-8 文本"));
        }
    }
    if !carry.is_empty() {
        return Err(AssetError::invalid_content(
            "pageText 不是合法 UTF-8 文本（结尾不完整）",
        ));
    }
    Ok(())
}

fn open(path: &Path) -> Result<std::fs::File, AssetError> {
    std::fs::File::open(path)
        .map_err(|error| AssetError::invalid_content(format!("无法读取上传内容：{error}")))
}

fn read_head(path: &Path, length: usize) -> Result<Vec<u8>, AssetError> {
    let mut file = open(path)?;
    let mut buffer = vec![0_u8; length];
    let mut filled = 0_usize;
    while filled < buffer.len() {
        let read = file
            .read(&mut buffer[filled..])
            .map_err(|error| AssetError::invalid_content(format!("读取内容失败：{error}")))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    Ok(buffer)
}

fn read_tail(file: &mut std::fs::File, total: u64, length: u64) -> Result<Vec<u8>, AssetError> {
    let length = length.min(total);
    file.seek(SeekFrom::Start(total - length))
        .map_err(|error| AssetError::invalid_content(format!("JPEG 定位失败：{error}")))?;
    let mut buffer = vec![0_u8; length as usize];
    file.read_exact(&mut buffer)
        .map_err(|error| AssetError::invalid_content(format!("JPEG 读取失败：{error}")))?;
    Ok(buffer)
}

fn read_at(
    file: &mut std::fs::File,
    offset: u64,
    length: usize,
    _scratch: &mut [u8],
) -> Result<Vec<u8>, AssetError> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| AssetError::invalid_content(format!("定位失败：{error}")))?;
    let mut buffer = vec![0_u8; length];
    file.read_exact(&mut buffer)
        .map_err(|error| AssetError::invalid_content(format!("读取失败：{error}")))?;
    Ok(buffer)
}

/// IEEE CRC-32（PNG chunk 校验；与 tests/fixtures 的独立实现同算法）。
struct Crc32 {
    state: u32,
}

impl Crc32 {
    fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(self.state & 1);
                self.state = (self.state >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "em-validate-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn non_pdf_for_document_purpose_is_unsupported_type() {
        let path = temp_file("fake.pdf", b"plain text pretending to be a pdf");
        let error = validate(&path, AssetPurpose::Document, &Limits::default()).unwrap_err();
        assert!(
            matches!(error, AssetError::UnsupportedType { .. }),
            "{error:?}"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn text_purpose_rejects_invalid_utf8() {
        let path = temp_file("page.txt", &[0x41, 0xFF, 0xFE, 0x42]);
        let error = validate(&path, AssetPurpose::PageText, &Limits::default()).unwrap_err();
        assert!(
            matches!(error, AssetError::InvalidContent { .. }),
            "{error:?}"
        );
        assert_eq!(
            validate(
                &temp_file("ok.txt", "页面文字".as_bytes()),
                AssetPurpose::PageText,
                &Limits::default()
            )
            .unwrap()
            .mime,
            "text/plain; charset=utf-8"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn photo_purpose_rejects_unknown_magic() {
        let path = temp_file("photo.png", b"RIFF....WEBPVP8 ");
        let error = validate(&path, AssetPurpose::Photo, &Limits::default()).unwrap_err();
        assert!(
            matches!(error, AssetError::UnsupportedType { .. }),
            "{error:?}"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn pixel_budget_is_enforced_with_details() {
        let error = check_pixel_budget(60_000, 60_000).unwrap_err();
        match error {
            AssetError::InvalidContent { details, .. } => {
                let details = details.expect("应带 details");
                assert_eq!(details["reason"], "imagePixels");
                assert_eq!(details["width"], 60_000);
            }
            other => panic!("应为 InvalidContent：{other:?}"),
        }
        assert!(check_pixel_budget(4000, 3000).is_ok());
    }

    #[test]
    fn purpose_limits_follow_the_purpose() {
        let limits = Limits::default();
        assert_eq!(
            purpose_limit(&limits, AssetPurpose::Document),
            50 * 1_048_576
        );
        assert_eq!(purpose_limit(&limits, AssetPurpose::Photo), 20 * 1_048_576);
        assert_eq!(
            purpose_limit(&limits, AssetPurpose::PageImage),
            20 * 1_048_576
        );
        assert_eq!(
            purpose_limit(&limits, AssetPurpose::PageText),
            PAGE_TEXT_MAX_BYTES
        );
        assert_eq!(purpose_limit(&limits, AssetPurpose::Model), 0);
        assert_eq!(
            max_upload_request_bytes(&limits),
            50 * 1_048_576 + MULTIPART_OVERHEAD_BYTES
        );
    }

    #[test]
    fn crc32_matches_known_value() {
        // "123456789" 的 IEEE CRC-32 标准测试向量。
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }
}
