//! 原创样例资产的**生成器**（刻意与校验器同 crate，便于"生成 → 结构校验 → sha256 断言"
//! 在一个测试进程内闭环）。
//!
//! 设计约束（T05 卡第 3 点）：
//! - 不拷贝任何第三方受版权素材；全部字节按公开规范（glTF 2.0 / PDF 1.4 / PNG /
//!   JPEG baseline）现场构造，许可见 `tests/fixtures/README.md`；
//! - **确定性输出**：无时间戳、无随机数；`fixture_harness.rs` 用固定 sha256 断言
//!   "重新生成的字节 == 仓库中提交的字节"；
//! - 命令行入口见 `src/bin/generate_fixtures.rs`。

use crate::assets::crc32_ieee;

/// 生成全部样例资产：`(文件名, 字节)`。
pub fn build_all() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("sample-model.glb", build_glb()),
        ("sample-manual-text.pdf", build_text_pdf()),
        ("sample-manual-scan.pdf", build_scan_pdf()),
        // T09：PDF 准备（REQ-014）需要的失败与边界样例。
        ("sample-manual-rotated.pdf", build_rotated_pdf()),
        ("sample-manual-nonlatin.pdf", build_nonlatin_pdf()),
        ("sample-manual-encrypted.pdf", build_encrypted_pdf()),
        ("sample-manual-many-pages.pdf", build_many_pages_pdf(101)),
        ("sample-photo-front.jpg", build_jpeg()),
        ("sample-photo-left.png", build_photo_png()),
    ]
}

// ---------------------------------------------------------------------------
// GLB：单位立方体（12 三角面）+ 16×16 内嵌 PNG 贴图，自包含 BIN
// ---------------------------------------------------------------------------

pub fn build_glb() -> Vec<u8> {
    let texture = build_texture_png(16);

    // 位置/法线/UV/索引/贴图依次放进 BIN，全部 4 字节对齐。
    let vertices = cube_vertices();
    let indices = cube_indices();
    let mut bin: Vec<u8> = Vec::new();
    let positions_offset = bin.len();
    for vertex in &vertices {
        for value in &vertex.position {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let normals_offset = bin.len();
    for vertex in &vertices {
        for value in &vertex.normal {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let uvs_offset = bin.len();
    for vertex in &vertices {
        for value in &vertex.uv {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    }
    let indices_offset = bin.len();
    for index in &indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let texture_offset = bin.len();
    bin.extend_from_slice(&texture);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let json = serde_json::json!({
        "asset": {
            "version": "2.0",
            "generator": "everything-manual test fixtures (self-built, T05)"
        },
        "scene": 0,
        "scenes": [ { "nodes": [ 0 ] } ],
        "nodes": [ { "mesh": 0, "name": "unit-cube" } ],
        "meshes": [ {
            "name": "cube",
            "primitives": [ {
                "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
                "indices": 3,
                "material": 0,
                "mode": 4
            } ]
        } ],
        "materials": [ {
            "name": "quadrants",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicFactor": 0.0,
                "roughnessFactor": 1.0
            }
        } ],
        "textures": [ { "sampler": 0, "source": 0 } ],
        "images": [ { "bufferView": 4, "mimeType": "image/png" } ],
        "samplers": [ { "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 } ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": positions_offset, "byteLength": vertices.len() * 12 },
            { "buffer": 0, "byteOffset": normals_offset, "byteLength": vertices.len() * 12 },
            { "buffer": 0, "byteOffset": uvs_offset, "byteLength": vertices.len() * 8 },
            { "buffer": 0, "byteOffset": indices_offset, "byteLength": indices.len() * 2 },
            { "buffer": 0, "byteOffset": texture_offset, "byteLength": texture.len() }
        ],
        "accessors": [
            {
                "bufferView": 0, "componentType": 5126, "count": vertices.len(), "type": "VEC3",
                "min": [ -1.0, -1.0, -1.0 ], "max": [ 1.0, 1.0, 1.0 ]
            },
            { "bufferView": 1, "componentType": 5126, "count": vertices.len(), "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": vertices.len(), "type": "VEC2" },
            { "bufferView": 3, "componentType": 5123, "count": indices.len(), "type": "SCALAR" }
        ],
        "buffers": [ { "byteLength": bin.len() } ]
    });

    let mut json_bytes = serde_json::to_vec(&json).expect("序列化 glTF JSON");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }

    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

/// 六个面各 4 个顶点（法线/UV 独立，便于后续阅读器与拾取测试）。
fn cube_vertices() -> Vec<Vertex> {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let combinations = [[-1.0_f32, -1.0_f32], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let mut vertices = Vec::new();
    for (normal, u, v) in faces {
        for (combination, uv) in combinations.iter().zip(uvs.iter()) {
            vertices.push(Vertex {
                position: [
                    normal[0] + u[0] * combination[0] + v[0] * combination[1],
                    normal[1] + u[1] * combination[0] + v[1] * combination[1],
                    normal[2] + u[2] * combination[0] + v[2] * combination[1],
                ],
                normal,
                uv: *uv,
            });
        }
    }
    vertices
}

fn cube_indices() -> Vec<u16> {
    let mut indices = Vec::new();
    for face in 0..6_u16 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    indices
}

/// 四象限 + 边框的方形贴图（PNG）。
pub fn build_texture_png(size: u32) -> Vec<u8> {
    build_png(size, size, |x, y| {
        let border = x == 0 || y == 0 || x == size - 1 || y == size - 1;
        if border {
            [24, 24, 24]
        } else if x < size / 2 && y < size / 2 {
            [200, 40, 40]
        } else if x >= size / 2 && y < size / 2 {
            [40, 180, 60]
        } else if x < size / 2 {
            [40, 80, 220]
        } else {
            [230, 200, 40]
        }
    })
}

// ---------------------------------------------------------------------------
// PNG（真彩色 8 位，zlib stored 块，确定性）
// ---------------------------------------------------------------------------

/// 64×64 左视图样例照片（PNG）：双轴渐变 + 左上角标记块，便于区分视图与方向。
pub fn build_photo_png() -> Vec<u8> {
    const SIZE: u32 = 64;
    build_png(SIZE, SIZE, |x, y| {
        if x < 8 && y < 8 {
            return [255, 255, 255];
        }
        [255 - (x * 4) as u8, 96 + (y * 2) as u8, 128]
    })
}

/// 生成真彩 PNG（每行过滤器 0，zlib 只用 stored 块 → 无压缩库依赖）。
pub fn build_png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * height * 3 + height) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let compressed = zlib_stored(&raw);

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: truecolor
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &compressed);
    push_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_chunk(png: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(chunk_type);
    png.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(chunk_type);
    crc_input.extend_from_slice(data);
    png.extend_from_slice(&crc32_ieee(&crc_input).to_be_bytes());
}

/// zlib（仅 deflate stored 块 + adler32）。
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut remaining = raw;
    while !remaining.is_empty() {
        let chunk = &remaining[..remaining.len().min(65_535)];
        let is_final = chunk.len() == remaining.len();
        out.push(u8::from(is_final));
        let length = chunk.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(chunk);
        remaining = &remaining[chunk.len()..];
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------------------
// JPEG（灰度 baseline，自建最小 Huffman 表，无第三方编码器）
// ---------------------------------------------------------------------------
//
// 每个 8×8 块只使用 DC 系数（AC 全零 → 立即 EOB），图像是 16 个灰阶方块。
// 自建 DC 表含类别 0..=3（长度 2 的码 00/01/10 + 长度 3 的码 110；全 1 码按规范
// 保留不用），块间 DC 差分固定为 ±5 → 类别 3，落在表内。

/// 生成灰度 baseline JPEG（32×32）。
pub fn build_jpeg() -> Vec<u8> {
    const WIDTH: u16 = 32;
    const HEIGHT: u16 = 32;
    let mut block_dc = Vec::new();
    for index in 0..16_i32 {
        let step = if index < 8 { index } else { 15 - index };
        block_dc.push(step * 5);
    }

    let mut jpeg = vec![0xFF, 0xD8];
    // APP0 / JFIF
    jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    jpeg.extend_from_slice(b"JFIF\0");
    jpeg.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    // DQT：全 8（AC 系数为 0；DC 反量化 = C * 8 → 灰度 = C + 128）
    jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    jpeg.extend_from_slice(&[8_u8; 64]);
    // SOF0：8 位精度、单分量
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    jpeg.extend_from_slice(&HEIGHT.to_be_bytes());
    jpeg.extend_from_slice(&WIDTH.to_be_bytes());
    jpeg.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
    // DHT：DC 表（class 0 / id 0）
    let dc_bits: [u8; 16] = [0, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let dc_symbols: [u8; 4] = [0, 1, 2, 3];
    jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, (2 + 1 + 16 + 4) as u8, 0x00]);
    jpeg.extend_from_slice(&dc_bits);
    jpeg.extend_from_slice(&dc_symbols);
    // DHT：AC 表（class 1 / id 0）——只含 EOB 符号 0x00
    let ac_bits: [u8; 16] = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, (2 + 1 + 16 + 1) as u8, 0x10]);
    jpeg.extend_from_slice(&ac_bits);
    jpeg.extend_from_slice(&[0x00]);
    // SOS
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);

    // 熵编码数据：DC 码（自建表）+ 振幅位 + AC 的 EOB。
    const DC_CODES: [(u8, u8); 4] = [(0b00, 2), (0b01, 2), (0b10, 2), (0b110, 3)];
    let mut bits = BitWriter::default();
    let mut previous = 0_i32;
    for dc in &block_dc {
        let diff = dc - previous;
        previous = *dc;
        let size = bit_length(diff);
        assert!(size <= 3, "JPEG 生成器只支持 DC 类别 0..=3，实际 {size}");
        let (code, code_len) = DC_CODES[size as usize];
        bits.push_bits(u32::from(code), code_len);
        bits.push_bits(amplitude_bits(diff, size), size);
        bits.push_bits(0b0, 1); // AC EOB
    }
    jpeg.extend_from_slice(&bits.finish());

    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

fn bit_length(value: i32) -> u8 {
    let mut magnitude = value.unsigned_abs();
    let mut size = 0;
    while magnitude > 0 {
        size += 1;
        magnitude >>= 1;
    }
    size
}

fn amplitude_bits(value: i32, size: u8) -> u32 {
    if value >= 0 {
        value as u32
    } else {
        (value + ((1_i32 << size) - 1)) as u32
    }
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    current: u8,
    bit_count: u8,
}

impl BitWriter {
    fn push_bits(&mut self, value: u32, count: u8) {
        for shift in (0..count).rev() {
            let bit = ((value >> shift) & 1) as u8;
            self.current = (self.current << 1) | bit;
            self.bit_count += 1;
            if self.bit_count == 8 {
                self.flush_byte();
            }
        }
    }

    fn flush_byte(&mut self) {
        let byte = self.current;
        self.out.push(byte);
        if byte == 0xFF {
            self.out.push(0x00); // 字节填充
        }
        self.current = 0;
        self.bit_count = 0;
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            let pad = 8 - self.bit_count;
            self.current = (self.current << pad) | ((1 << pad) - 1);
            self.flush_byte();
        }
        self.out
    }
}

// ---------------------------------------------------------------------------
// PDF（最小 PDF 1.4：text 版含文字层；scan 版页图为无压缩灰度栅格）
// ---------------------------------------------------------------------------

/// 文字型 PDF：2 页、Helvetica 标准字体、无压缩内容流。
pub fn build_text_pdf() -> Vec<u8> {
    let lines_page_1 = [
        "Sample Manual - Synthetic Fixture (Page 1 of 2)",
        "Model X100: self-built test document, not a real manual.",
        "Step 1: Loosen the four captive screws on the rear cover.",
    ];
    let lines_page_2 = [
        "Sample Manual - Synthetic Fixture (Page 2 of 2)",
        "Step 2: Route the sensor cable away from the heat sink.",
        "Specification: DC 12 V, 2.5 A, operating range 0-40 C.",
    ];
    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    pdf.raw(b"<< /Type /Pages /Kids [ 3 0 R 5 0 R ] /Count 2 >>");
    pdf.end_object();
    pdf.begin_object(3);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(4);
    let content = page_content(&lines_page_1);
    pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.raw(content.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(5);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(6);
    let content = page_content(&lines_page_2);
    pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.raw(content.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(7);
    pdf.raw(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    pdf.end_object();
    pdf.finish(1)
}

fn page_content(lines: &[&str]) -> String {
    let mut content = String::from("BT\n/F1 18 Tf\n72 760 Td\n");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            content.push_str("0 -28 Td\n");
        }
        let escaped = line
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        content.push_str(&format!("({escaped}) Tj\n"));
    }
    content.push_str("ET\n");
    content
}

/// 扫描型 PDF：2 页，页图为无压缩 8 位灰度栅格，**无文字层**。
///
/// 像素值刻意取自 {16,32,48,64,80,96,112}（无 ASCII 字母），保证文件是纯 ASCII：
/// 结构校验按字节偏移解析时不受编码转换影响，也避免页图数据里出现 `BT`/`endstream`。
pub fn build_scan_pdf() -> Vec<u8> {
    const WIDTH: usize = 32;
    const HEIGHT: usize = 32;
    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    pdf.raw(b"<< /Type /Pages /Kids [ 3 0 R 6 0 R ] /Count 2 >>");
    pdf.end_object();

    for (page_index, (page_object, content_object, image_object)) in
        [(3_usize, 4_usize, 5_usize), (6, 7, 8)]
            .into_iter()
            .enumerate()
    {
        pdf.begin_object(page_object);
        pdf.raw(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
                  /Resources << /XObject << /Im0 {image_object} 0 R >> >> \
                  /Contents {content_object} 0 R >>"
            )
            .as_bytes(),
        );
        pdf.end_object();

        pdf.begin_object(content_object);
        let content = "q 595 0 0 842 0 0 cm /Im0 Do Q\n";
        pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        pdf.raw(content.as_bytes());
        pdf.raw(b"endstream");
        pdf.end_object();

        let pixels = scan_page_pixels(page_index, WIDTH, HEIGHT);
        pdf.begin_object(image_object);
        pdf.raw(
            format!(
                "<< /Type /XObject /Subtype /Image /Width {WIDTH} /Height {HEIGHT} \
                  /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n",
                pixels.len()
            )
            .as_bytes(),
        );
        pdf.raw(&pixels);
        pdf.raw(b"\nendstream");
        pdf.end_object();
    }

    pdf.finish(1)
}

/// 页图像素（确定性图案，取值全部 < 128 且不含 ASCII 字母）。
fn scan_page_pixels(page_index: usize, width: usize, height: usize) -> Vec<u8> {
    const LEVELS: [u8; 7] = [16, 32, 48, 64, 80, 96, 112];
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let index = (x / 4 + 2 * (y / 4) + 5 * page_index) % LEVELS.len();
            pixels.push(LEVELS[index]);
        }
    }
    pixels
}

// ---------------------------------------------------------------------------
// T09：PDF 准备的边界样例（旋转页 / 非拉丁字体 / 加密 / 101 页）
// ---------------------------------------------------------------------------

/// 旋转页 PDF：2 页，第 2 页带 `/Rotate 90`。
///
/// 用途（AC-022/AC-023）：验证页图按**旋转后 viewport** 渲染（第 2 页宽高互换），
/// 页图坐标原点为旋转后 viewport 左上角。文字与 `sample-manual-text.pdf` 不同，
/// e2e 可按文字断言取到的是这一份。
pub fn build_rotated_pdf() -> Vec<u8> {
    let lines_page_1 = [
        "Rotated fixture (page 1 of 2): upright page.",
        "Marker: ROTATE-PAGE-ONE",
    ];
    let lines_page_2 = [
        "Rotated fixture (page 2 of 2): this page is /Rotate 90.",
        "Marker: ROTATE-PAGE-TWO",
    ];
    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    pdf.raw(b"<< /Type /Pages /Kids [ 3 0 R 5 0 R ] /Count 2 >>");
    pdf.end_object();
    pdf.begin_object(3);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(4);
    let content = page_content(&lines_page_1);
    pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.raw(content.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(5);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] /Rotate 90 \
          /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(6);
    let content = page_content(&lines_page_2);
    pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.raw(content.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(7);
    pdf.raw(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    pdf.end_object();
    pdf.finish(1)
}

/// 非拉丁字体 PDF：1 页，Type0/CID 字体 + `UniGB-UCS2-H` 编码 + ToUnicode CMap。
///
/// 为什么要这个样例（AC-023、架构 §5.1）：
/// - 非 Identity 的 CMap 需要 PDF.js 从 `cMapUrl` **本地**读取 `UniGB-UCS2-H.bcmap`，
///   因此它同时证明"CMaps 不访问 CDN"这条约束（e2e 断言该本地请求发生且无外网请求）；
/// - 文字层是 CJK（非拉丁），提取必须正确（e2e 断言页文字资产内容含中文）。
pub fn build_nonlatin_pdf() -> Vec<u8> {
    // 文本（按 UCS-2 码位写成 2 字节十六进制串；UniGB-UCS2-H 的输入码 = UCS-2）。
    let text = "部件一：松开四颗螺丝";
    let mut hex = String::new();
    let mut to_unicode = String::new();
    for character in text.chars() {
        let code = character as u32;
        assert!(code <= 0xFFFF, "样例只使用 BMP 字符");
        hex.push_str(&format!("{code:04X}"));
        to_unicode.push_str(&format!("<{code:04X}> <{code:04X}>\n"));
    }
    let content = format!("BT\n/F1 24 Tf\n72 700 Td\n<{hex}> Tj\nET\n");
    let char_count = text.chars().count();
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n\
         {char_count} beginbfchar\n\
         {to_unicode}endbfchar\n\
         endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n"
    );
    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    pdf.raw(b"<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>");
    pdf.end_object();
    pdf.begin_object(3);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(4);
    // 无内嵌字体程序（BaseFont 是 Adobe-GB1 的标准字体名）：PDF.js 用标准字体/替代字体
    // 绘制字形，文字提取走 ToUnicode —— 本样例关心的是"提取正确 + CMap 本地加载"。
    pdf.raw(
        b"<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light \
          /Encoding /UniGB-UCS2-H /DescendantFonts [ 6 0 R ] /ToUnicode 7 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(5);
    pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.raw(content.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(6);
    pdf.raw(
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> \
          /FontDescriptor 8 0 R /DW 1000 >>",
    );
    pdf.end_object();
    pdf.begin_object(7);
    pdf.raw(format!("<< /Length {} >>\nstream\n", cmap.len()).as_bytes());
    pdf.raw(cmap.as_bytes());
    pdf.raw(b"endstream");
    pdf.end_object();
    pdf.begin_object(8);
    pdf.raw(
        b"<< /Type /FontDescriptor /FontName /STSong-Light /Flags 4 \
          /FontBBox [ 0 -150 1000 900 ] /ItalicAngle 0 /Ascent 900 /Descent -150 \
          /CapHeight 900 /StemV 80 >>",
    );
    pdf.end_object();
    pdf.finish(1)
}

/// 101 页 PDF（超过 100 页上限；AC-024 的"超页数拒绝"样例）。
///
/// 每页只有一行文字，整体体积很小；页数刻意取 101（上限 +1）而不是更大，
/// 保证"拒绝原因里的实际页数"可被逐字核对。
///
/// **页数用间接引用给出**（`/Count 3 0 R`，对象 3 的值是 101）。原因：
/// - T06 的上传探针（`crates/server/src/assets/pdf.rs`）刻意只做廉价文本读取，
///   不解析间接引用 —— 它把该文件判为"页数 3"而放行（`Parsed{3}`）；
/// - PDF.js 会解析间接引用，看到 101 页，于是 **T09 准备阶段**给出权威拒绝
///   （UI-016 的「PDF 共 N 页，超过 100 页上限」）。这正是 REQ-014 设计的分工：
///   上传层是第一道防线、准备阶段是权威判定（见 ADR-003 与 pdf.rs 模块注释）。
/// - 本仓库的结构校验器（`crate::assets::validate_pdf`）会跟随一次间接引用，
///   因此自检仍能核对"声明页数 == 实际页数"。
pub fn build_many_pages_pdf(page_count: usize) -> Vec<u8> {
    assert!(page_count >= 1);
    // 对象编号：1=Catalog，2=Pages，3=/Count 的间接数值对象，之后每页两个对象。
    let count_object = 3_usize;
    let page_objects: Vec<usize> = (0..page_count).map(|index| 4 + index * 2).collect();
    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    let kids = page_objects
        .iter()
        .map(|number| format!("{number} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    pdf.raw(format!("<< /Type /Pages /Count {count_object} 0 R /Kids [ {kids} ] >>").as_bytes());
    pdf.end_object();
    pdf.begin_object(count_object);
    pdf.raw(page_count.to_string().as_bytes());
    pdf.end_object();

    let font_object = 4 + page_count * 2;
    for (index, page_object) in page_objects.iter().enumerate() {
        let content_object = page_object + 1;
        let page_number = index + 1;
        pdf.begin_object(*page_object);
        pdf.raw(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
                  /Resources << /Font << /F1 {font_object} 0 R >> >> /Contents {content_object} 0 R >>"
            )
            .as_bytes(),
        );
        pdf.end_object();
        pdf.begin_object(content_object);
        let content = page_content(&[&format!(
            "Many pages fixture: page {page_number} of {page_count}."
        )]);
        pdf.raw(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        pdf.raw(content.as_bytes());
        pdf.raw(b"endstream");
        pdf.end_object();
    }
    pdf.begin_object(font_object);
    pdf.raw(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    pdf.end_object();
    pdf.finish(1)
}

/// **真正加密**的 PDF（标准安全处理器 V=1 / R=2，RC4 40 位，用户口令非空）。
///
/// 用途（AC-024）：PDF.js 在未提供口令时抛 `PasswordException(NEED_PASSWORD)`，
/// 应用必须拒绝并给出可行动文案，且不创建任何记录。
///
/// 这里是按 PDF 规范算法 2/3/4/5 现场实现的：口令填充、O/U 计算、文件密钥派生、
/// 内容流 RC4 加密。因此带正确口令的解析器可以正常打开，只有"无口令"才被拒——
/// 用它证明的是"真的加密"，而不是"看起来像加密"。
pub fn build_encrypted_pdf() -> Vec<u8> {
    const USER_PASSWORD: &str = "fixture-secret";
    const FILE_ID: [u8; 16] = [
        0x54, 0x09, 0x2A, 0xE1, 0x33, 0x7B, 0x48, 0xD0, 0x9F, 0x11, 0x6C, 0xBE, 0x22, 0x40, 0x7A,
        0x8D,
    ];
    let padding = PASSWORD_PADDING;
    let padded_user = pad_password(USER_PASSWORD.as_bytes(), &padding);
    // O：用 owner 口令（此样例与 user 口令相同）派生密钥并加密填充后的 user 口令。
    let owner_key = md5(&padded_user)[..5].to_vec();
    let o = rc4(&owner_key, &padded_user);
    // 文件密钥：MD5(pad(user) + O + P(4B LE) + ID[0])[0..5]；P = -1（全部权限）。
    let mut key_input = Vec::new();
    key_input.extend_from_slice(&padded_user);
    key_input.extend_from_slice(&o);
    key_input.extend_from_slice(&(-1_i32).to_le_bytes());
    key_input.extend_from_slice(&FILE_ID);
    let file_key = md5(&key_input)[..5].to_vec();
    // U：用文件密钥加密口令填充串。
    let u = rc4(&file_key, &padding);

    let mut pdf = PdfBuilder::new();
    pdf.begin_object(1);
    pdf.raw(b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.end_object();
    pdf.begin_object(2);
    pdf.raw(b"<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>");
    pdf.end_object();
    pdf.begin_object(3);
    pdf.raw(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 6 0 R >> >> /Contents 4 0 R >>",
    );
    pdf.end_object();
    pdf.begin_object(4);
    // 内容流按 R=2 规则整体 RC4 加密（字符串与流共用同一文件密钥）。
    let mut content = Vec::new();
    content.extend_from_slice(b"BT\n/F1 18 Tf\n72 760 Td\n(");
    content.extend_from_slice(b"Encrypted fixture page (password protected).");
    content.extend_from_slice(b") Tj\nET\n");
    let encrypted_content = rc4(&file_key, &content);
    pdf.raw(format!("<< /Length {} >>\nstream\n", encrypted_content.len()).as_bytes());
    pdf.raw(&encrypted_content);
    pdf.raw(b"\nendstream");
    pdf.end_object();
    pdf.begin_object(5);
    pdf.raw(
        format!(
            "<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P -1 >>",
            hex_lower(&o),
            hex_lower(&u)
        )
        .as_bytes(),
    );
    pdf.end_object();
    pdf.begin_object(6);
    pdf.raw(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    pdf.end_object();
    // 加密文件必须带 /Encrypt 与 /ID（密钥派生依赖 ID[0]）。
    pdf.finish_with_encrypt(1, 5, &FILE_ID)
}

/// PDF 标准安全处理器的口令填充串（规范 Table 中的 32 字节常量）。
const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// 把口令按规范填充/截断到 32 字节。
fn pad_password(password: &[u8], padding: &[u8; 32]) -> [u8; 32] {
    let mut padded = [0_u8; 32];
    let take = password.len().min(32);
    padded[..take].copy_from_slice(&password[..take]);
    padded[take..].copy_from_slice(&padding[..32 - take]);
    padded
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// RC4（PDF R=2 的流/字符串加密算法；样例规模下的直接实现）。
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    assert!(!key.is_empty());
    let mut state: [u8; 256] = core::array::from_fn(|index| index as u8);
    let mut j = 0_u8;
    for i in 0..256 {
        j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
        state.swap(i, j as usize);
    }
    let mut out = Vec::with_capacity(data.len());
    let (mut i, mut j) = (0_u8, 0_u8);
    for byte in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(state[i as usize]);
        state.swap(i as usize, j as usize);
        let k = state[(state[i as usize].wrapping_add(state[j as usize])) as usize];
        out.push(byte ^ k);
    }
    out
}

/// MD5（RFC 1321）：标准安全处理器派生密钥需要它；样例不引入第三方依赖。
fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] =
        core::array::from_fn(|index| ((index as f64 + 1.0).sin().abs() * 4294967296.0) as u32);

    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    let (mut a0, mut b0, mut c0, mut d0) = (
        0x67452301_u32,
        0xEFCDAB89_u32,
        0x98BADCFE_u32,
        0x10325476_u32,
    );
    for chunk in message.as_chunks::<64>().0 {
        let mut m = [0_u32; 16];
        for (index, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for index in 0..64 {
            let (f, g) = match index {
                0..=15 => ((b & c) | (!b & d), index),
                16..=31 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(k[index])
                .wrapping_add(m[g])
                .rotate_left(S[index]);
            b = b.wrapping_add(rotated);
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut digest = [0_u8; 16];
    digest[..4].copy_from_slice(&a0.to_le_bytes());
    digest[4..8].copy_from_slice(&b0.to_le_bytes());
    digest[8..12].copy_from_slice(&c0.to_le_bytes());
    digest[12..].copy_from_slice(&d0.to_le_bytes());
    digest
}

struct PdfBuilder {
    out: Vec<u8>,
    /// `offsets[n]` 为对象 n 的字节偏移（下标 0 未用）。
    offsets: Vec<usize>,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            out: b"%PDF-1.4\n".to_vec(),
            offsets: vec![0],
        }
    }

    /// 开始第 `number` 个对象（必须按 1、2、3… 顺序）。
    fn begin_object(&mut self, number: usize) {
        assert_eq!(number, self.offsets.len(), "对象编号必须连续");
        self.offsets.push(self.out.len());
        self.out
            .extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn end_object(&mut self) {
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// 写 xref 表 + trailer + `%%EOF`；条目固定 20 字节（与规范一致）。
    fn finish(&mut self, root: usize) -> Vec<u8> {
        self.finish_with_encrypt(root, 0, &[0_u8; 16])
    }

    /// 同 [`PdfBuilder::finish`]，但 trailer 带 `/Encrypt` 与 `/ID`（加密样例专用）。
    fn finish_with_encrypt(&mut self, root: usize, encrypt: usize, file_id: &[u8; 16]) -> Vec<u8> {
        let xref_offset = self.out.len();
        let size = self.offsets.len();
        self.out
            .extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        self.out.extend_from_slice(b"0000000000 65535 f \n");
        for number in 1..size {
            self.out
                .extend_from_slice(format!("{:010} 00000 n \n", self.offsets[number]).as_bytes());
        }
        let extra = if encrypt == 0 {
            String::new()
        } else {
            format!(
                " /Encrypt {encrypt} 0 R /ID [ <{}> <{}> ]",
                hex_lower(file_id),
                hex_lower(file_id)
            )
        };
        self.out.extend_from_slice(
            format!(
                "trailer\n<< /Size {size} /Root {root} 0 R{extra} >>\nstartxref\n{xref_offset}\n%%EOF\n"
            )
            .as_bytes(),
        );
        std::mem::take(&mut self.out)
    }
}
