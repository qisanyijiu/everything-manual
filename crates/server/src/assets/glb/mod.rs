//! GLB 结构校验与能力清单（T13 / REQ-028；contracts.md §7）。
//!
//! 这是**服务端 CPU 结构校验**：判断"字节是不是一个自包含、可交付给浏览器的
//! glTF 2.0 二进制"以及是否在产品预算内（默认三角面 ≤100000、贴图单边 ≤4096，
//! 见 [`GlbBudget`]）。它**不**证明 GPU 一定能成功绘制（渲染在浏览器，T18 复核），
//! 也不修改模型字节（校验失败保留原始模型与错误，不静默改坏模型）。
//!
//! 检查清单（contracts.md §7 逐项）：
//!
//! | 项 | 规则 |
//! | --- | --- |
//! | magic/version/声明长度 | `glTF` + version=2 + 头内 `length` 必须等于文件实际字节数 |
//! | chunk 长度 | 首 chunk 必须是 JSON；可选第二 chunk 必须是 BIN；长度 4 字节对齐且不越界 |
//! | JSON 结构 | `asset.version == "2.0"`；字段类型错误明确报错（不猜测） |
//! | bufferView/accessor 范围 | byteOffset/byteLength/byteStride/componentType/count 全部在界内 |
//! | 索引边界 | 索引值必须 < 对应 POSITION 顶点数（逐值读取 BIN 判定） |
//! | 有限坐标 | POSITION 数据逐值检查 finite（NaN/Infinity 拒绝） |
//! | 非空几何 | 至少一个三角面；POSITION 必须存在且为 FLOAT VEC3 |
//! | 资源内嵌 | buffer 不得带 `uri`（必须用 BIN chunk）；image 不得带 `uri`（必须用 bufferView） |
//! | 扩展 | `extensionsRequired` 必须为空（首版不支持任何 required extension） |
//! | 预算 | 三角面 ≤ `max_triangles`；贴图单边 ≤ `max_texture_dimension` |
//!
//! 明确不支持（拒绝而不是猜测；能力清单见 `implementation.md` §T13）：
//! - `sparse` accessor；`data:` URI 资源（含 buffer/image）；GLB 的额外/未知 chunk 类型；
//! - `extensionsRequired` 中的任何扩展（例如 Draco 压缩）。
//!
//! 读取策略：容器与 JSON 一次性读入（上限由 GLB 总体积上限约束），BIN 中的坐标/索引
//! 数据**分块读取**（不把整个 BIN 读进内存）；所有读取都在已校验的范围之内。

pub mod download;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

pub use download::{
    DownloadError, DownloadPolicy, DownloadedModel, HostResolver, ModelDownloader, ResolveFuture,
    SystemHostResolver,
};

/// 产品预算（contracts.md §7 默认值；测试可注入更小的预算以覆盖超限路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlbBudget {
    /// 三角面上限（默认 100000）。
    pub max_triangles: u64,
    /// 贴图单边像素上限（默认 4096）。
    pub max_texture_dimension: u32,
}

impl Default for GlbBudget {
    fn default() -> Self {
        Self {
            max_triangles: DEFAULT_MAX_TRIANGLES,
            max_texture_dimension: DEFAULT_MAX_TEXTURE_DIMENSION,
        }
    }
}

/// 默认三角面上限（contracts.md §7）。
pub const DEFAULT_MAX_TRIANGLES: u64 = 100_000;
/// 默认贴图单边上限（contracts.md §7）。
pub const DEFAULT_MAX_TEXTURE_DIMENSION: u32 = 4096;

/// 校验错误（`code()` 是稳定标识：测试断言、`needs_input.items[].code` 与日志共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlbError {
    /// 容器级问题（magic/version/声明长度/chunk 布局/JSON 不可解析）。
    Container { code: &'static str, detail: String },
    /// glTF JSON 语义问题（asset.version、required extension、外链 URI、范围不一致、不支持特性）。
    Structure { code: &'static str, detail: String },
    /// 几何数据问题（索引越界、非有限坐标、空几何、缺 POSITION）。
    Geometry { code: &'static str, detail: String },
    /// 超出产品预算（面数/贴图尺寸）→ 上层进入 `needs_input`，保留原始模型与错误。
    Budget { code: &'static str, detail: String },
    /// 读取文件失败（IO；不泄露磁盘路径以外的内容）。
    Io { detail: String },
}

impl GlbError {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Container { code, .. } | Self::Structure { code, .. } => code,
            Self::Geometry { code, .. } => code,
            Self::Budget { code, .. } => code,
            Self::Io { .. } => "glb_io",
        }
    }

    /// 是否属于"超出产品预算"（上层提示"更换资料/重新生成或按 needs_input 补齐"，
    /// 不自动降预算）。
    pub fn is_budget(&self) -> bool {
        matches!(self, Self::Budget { .. })
    }

    /// 面向用户的说明（中文、可行动；不含磁盘路径与内部细节）。
    pub fn message(&self) -> String {
        match self {
            Self::Container { detail, .. } | Self::Structure { detail, .. } => detail.clone(),
            Self::Geometry { detail, .. } => detail.clone(),
            Self::Budget { detail, .. } => detail.clone(),
            Self::Io { detail } => format!("读取模型文件失败：{detail}"),
        }
    }

    /// 日志用一行摘要（与用户消息相同；错误里不含磁盘路径与密钥）。
    pub fn log_summary(&self) -> String {
        format!("{}：{}", self.code(), self.message())
    }
}

impl std::fmt::Display for GlbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.log_summary())
    }
}

impl std::error::Error for GlbError {}

/// 校验通过的模型摘要（写入 `model_revisions.bounds` 与阶段 usage）。
#[derive(Debug, Clone, PartialEq)]
pub struct GlbSummary {
    pub triangles: u64,
    pub vertices: u64,
    pub meshes: usize,
    pub primitives: usize,
    pub images: usize,
    pub max_texture_dimension: u32,
    /// 全部 POSITION 数据的轴对齐包围盒（asset-root 局部坐标；有限值）。
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

impl GlbSummary {
    /// `bounds` 的 JSON 表示（`model_revisions.bounds`）。
    pub fn bounds_json(&self) -> Value {
        serde_json::json!({
            "min": [self.bounds_min[0], self.bounds_min[1], self.bounds_min[2]],
            "max": [self.bounds_max[0], self.bounds_max[1], self.bounds_max[2]],
            "triangles": self.triangles,
            "vertices": self.vertices,
        })
    }
}

/// 校验一个 GLB 文件（`limit` 由调用方给出：与 `limits.max_glb_bytes` 一致）。
pub fn inspect_glb_file(path: &Path, budget: &GlbBudget) -> Result<GlbSummary, GlbError> {
    let mut file = std::fs::File::open(path).map_err(|error| GlbError::Io {
        detail: error.to_string(),
    })?;
    let len = file
        .metadata()
        .map_err(|error| GlbError::Io {
            detail: error.to_string(),
        })?
        .len();
    inspect(&mut file, len, budget)
}

/// 校验一个 GLB 读取器（测试可直接喂字节；`len` 为实际字节数）。
pub fn inspect<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    budget: &GlbBudget,
) -> Result<GlbSummary, GlbError> {
    let (json_chunk_len, bin) = read_container(reader, len)?;
    let gltf: Gltf = serde_json::from_slice(&read_exact_at(
        reader,
        JSON_CHUNK_DATA_OFFSET,
        json_chunk_len as usize,
    )?)
    .map_err(|error| GlbError::Container {
        code: "glb_json",
        detail: format!("GLB 的 JSON chunk 不是合法的 glTF 2.0 JSON：{error}"),
    })?;

    check_asset_and_extensions(&gltf)?;
    let layout = Layout::new(&gltf, bin)?;
    let geometry = collect_geometry(&gltf, &layout)?;
    check_budget(&geometry, budget)?;
    let images = check_images(reader, &gltf, &layout, budget)?;
    let bounds = check_positions(reader, &layout, &geometry)?;
    check_indices(reader, &layout, &geometry)?;

    Ok(GlbSummary {
        triangles: geometry.iter().map(|p| p.triangles).sum(),
        vertices: geometry.iter().map(|p| p.position_count).sum(),
        meshes: gltf.meshes.len(),
        primitives: geometry.len(),
        images: gltf.images.len(),
        max_texture_dimension: images
            .iter()
            .map(|image| image.max_dimension)
            .max()
            .unwrap_or(0),
        bounds_min: bounds.0,
        bounds_max: bounds.1,
    })
}

// ---------------------------------------------------------------------------
// 容器（header + chunk）
// ---------------------------------------------------------------------------

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: &[u8; 4] = b"JSON";
const CHUNK_BIN: &[u8; 4] = b"BIN\0";
const GLB_HEADER_LEN: u64 = 12;
const CHUNK_HEADER_LEN: u64 = 8;
/// 首个 chunk 数据开始处的文件偏移。
const JSON_CHUNK_DATA_OFFSET: u64 = GLB_HEADER_LEN + CHUNK_HEADER_LEN;

/// 读取并校验容器：返回 `(JSON chunk 长度, BIN chunk 数据在文件中的绝对偏移与长度)`。
#[allow(clippy::type_complexity)]
fn read_container<R: Read + Seek>(
    reader: &mut R,
    len: u64,
) -> Result<(u64, Option<(u64, u64)>), GlbError> {
    // 最小合法 GLB = 12 字节头 + 8 字节 chunk 头 + 至少 4 字节（非空 JSON chunk）。
    if len < JSON_CHUNK_DATA_OFFSET + 4 {
        return Err(GlbError::Container {
            code: "glb_magic",
            detail: format!("文件过短（{len} 字节），不是合法的 GLB"),
        });
    }
    let header = read_exact_at(reader, 0, 12)?;
    if &header[0..4] != GLB_MAGIC {
        return Err(GlbError::Container {
            code: "glb_magic",
            detail: "文件头不是 glTF magic（不是 GLB 二进制格式）".to_owned(),
        });
    }
    let version = u32::from_le_bytes(header[4..8].try_into().expect("4 字节"));
    if version != 2 {
        return Err(GlbError::Container {
            code: "glb_version",
            detail: format!("只支持 glTF 2.0（GLB version=2），实际 version={version}"),
        });
    }
    let declared = u32::from_le_bytes(header[8..12].try_into().expect("4 字节")) as u64;
    if declared != len {
        return Err(GlbError::Container {
            code: "glb_declared_length",
            detail: format!(
                "GLB 头部声明的长度（{declared} 字节）与实际文件大小（{len} 字节）不符（文件被截断或多出数据）"
            ),
        });
    }

    let mut offset = GLB_HEADER_LEN;
    let mut chunk_index = 0_usize;
    let mut json_len: Option<u64> = None;
    let mut bin: Option<(u64, u64)> = None;
    while offset < len {
        if offset + CHUNK_HEADER_LEN > len {
            return Err(GlbError::Container {
                code: "glb_chunk_layout",
                detail: "GLB chunk 头不完整（文件被截断）".to_owned(),
            });
        }
        let chunk_header = read_exact_at(reader, offset, CHUNK_HEADER_LEN as usize)?;
        let length = u32::from_le_bytes(chunk_header[0..4].try_into().expect("4 字节")) as u64;
        let kind: [u8; 4] = chunk_header[4..8].try_into().expect("4 字节");
        let data_offset = offset + CHUNK_HEADER_LEN;
        if !length.is_multiple_of(4) {
            return Err(GlbError::Container {
                code: "glb_chunk_layout",
                detail: format!(
                    "第 {chunk_index} 个 chunk 的长度（{length} 字节）不是 4 字节的整数倍"
                ),
            });
        }
        if data_offset + length > len {
            return Err(GlbError::Container {
                code: "glb_chunk_layout",
                detail: format!(
                    "第 {chunk_index} 个 chunk 声明 {length} 字节，超出文件范围（chunk 长度与文件不符）"
                ),
            });
        }
        match (chunk_index, &kind) {
            (0, kind) if kind == CHUNK_JSON => {
                if length == 0 {
                    return Err(GlbError::Container {
                        code: "glb_chunk_layout",
                        detail: "GLB 的 JSON chunk 长度为 0".to_owned(),
                    });
                }
                json_len = Some(length);
            }
            (0, _) => {
                return Err(GlbError::Container {
                    code: "glb_chunk_layout",
                    detail: "GLB 的第一个 chunk 必须是 JSON".to_owned(),
                });
            }
            (1, kind) if kind == CHUNK_BIN => {
                bin = Some((data_offset, length));
            }
            (_, _) => {
                let label = String::from_utf8_lossy(&kind).to_string();
                return Err(GlbError::Container {
                    code: "glb_chunk_layout",
                    detail: format!(
                        "出现不支持的 chunk（第 {chunk_index} 个，类型 {label:?}）：GLB 只接受 JSON + BIN 两个 chunk"
                    ),
                });
            }
        }
        offset = data_offset + length;
        chunk_index += 1;
    }

    let json_len = json_len.ok_or_else(|| GlbError::Container {
        code: "glb_chunk_layout",
        detail: "GLB 缺少 JSON chunk".to_owned(),
    })?;
    Ok((json_len, bin))
}

// ---------------------------------------------------------------------------
// glTF JSON
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Gltf {
    asset: Option<AssetInfo>,
    #[serde(default)]
    buffers: Vec<BufferDef>,
    #[serde(default)]
    buffer_views: Vec<BufferViewDef>,
    #[serde(default)]
    accessors: Vec<AccessorDef>,
    #[serde(default)]
    meshes: Vec<MeshDef>,
    #[serde(default)]
    images: Vec<ImageDef>,
    extensions_required: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetInfo {
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferDef {
    byte_length: Option<i64>,
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferViewDef {
    buffer: Option<i64>,
    byte_offset: Option<i64>,
    byte_length: Option<i64>,
    byte_stride: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessorDef {
    buffer_view: Option<i64>,
    byte_offset: Option<i64>,
    component_type: Option<i64>,
    count: Option<i64>,
    #[serde(rename = "type")]
    type_: Option<String>,
    /// 出现即拒绝（首版不支持 sparse）。
    sparse: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct MeshDef {
    primitives: Option<Vec<PrimitiveDef>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimitiveDef {
    attributes: Option<BTreeMap<String, i64>>,
    indices: Option<i64>,
    mode: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageDef {
    uri: Option<String>,
    buffer_view: Option<i64>,
    mime_type: Option<String>,
}

fn check_asset_and_extensions(gltf: &Gltf) -> Result<(), GlbError> {
    let version = gltf
        .asset
        .as_ref()
        .and_then(|asset| asset.version.as_deref());
    if version != Some("2.0") {
        return Err(GlbError::Structure {
            code: "gltf_asset_version",
            detail: format!(
                "glTF 版本不受支持：asset.version 必须是 \"2.0\"，实际 {}",
                version.unwrap_or("(缺失)")
            ),
        });
    }
    if let Some(required) = &gltf.extensions_required
        && !required.is_empty()
    {
        return Err(GlbError::Structure {
            code: "gltf_required_extension",
            detail: format!(
                "模型要求服务端不支持的扩展（extensionsRequired=[{}]）：首版不支持任何 required extension，\
                 请更换资料或重新生成不含该扩展的模型（不静默忽略）",
                required.join(", ")
            ),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 布局解析（buffer/bufferView/accessor 的范围与对齐）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct BufferViewSpan {
    /// 在 BIN chunk 数据内的相对偏移。
    offset: u64,
    length: u64,
    stride: Option<u64>,
}

/// 已校验的 glTF 布局：BIN chunk 位置 + 每个 bufferView 的可用范围。
struct Layout<'a> {
    /// BIN chunk 数据在文件中的绝对偏移（无 BIN chunk 时为 `None`）。
    bin_file_offset: Option<u64>,
    bin_len: u64,
    views: Vec<BufferViewSpan>,
    gltf: &'a Gltf,
}

impl<'a> Layout<'a> {
    fn new(gltf: &'a Gltf, bin: Option<(u64, u64)>) -> Result<Self, GlbError> {
        let (bin_file_offset, bin_len) = match bin {
            Some((offset, length)) => (Some(offset), length),
            None => (None, 0),
        };

        // buffers：必须内嵌（不得带 uri），且最多一个（GLB 只允许 buffer 0）。
        for (index, buffer) in gltf.buffers.iter().enumerate() {
            if buffer.uri.is_some() {
                return Err(GlbError::Structure {
                    code: "gltf_external_uri",
                    detail: format!(
                        "buffers[{index}] 带 uri（外部引用或 data: URI）：首版只接受 GLB 内 BIN chunk 内嵌资源"
                    ),
                });
            }
        }
        if gltf.buffers.len() > 1 {
            return Err(GlbError::Structure {
                code: "gltf_buffer_range",
                detail: format!(
                    "buffers 有 {} 个条目：GLB 只允许一个内嵌 buffer（buffer 0 = BIN chunk）",
                    gltf.buffers.len()
                ),
            });
        }
        if let Some(buffer) = gltf.buffers.first() {
            let Some(byte_length) = buffer.byte_length else {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: "buffers[0] 缺少 byteLength".to_owned(),
                });
            };
            if byte_length < 0 {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!("buffers[0].byteLength 非法：{byte_length}"),
                });
            }
            let byte_length = byte_length as u64;
            if byte_length > bin_len {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!(
                        "buffers[0].byteLength（{byte_length} 字节）超过 BIN chunk 的实际长度（{bin_len} 字节）"
                    ),
                });
            }
            // BIN chunk 按 4 字节补齐：允许 (bin_len - byteLength) ∈ 0..4 的填充。
            if bin_len - byte_length >= 4 {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!(
                        "BIN chunk 长度（{bin_len} 字节）明显大于 buffers[0].byteLength（{byte_length} 字节）"
                    ),
                });
            }
        }

        // bufferViews：必须在 buffer 范围内；byteStride 合法（4..=252 且 4 的倍数）。
        let mut views = Vec::with_capacity(gltf.buffer_views.len());
        for (index, view) in gltf.buffer_views.iter().enumerate() {
            let buffer = view.buffer.unwrap_or(0);
            if buffer != 0 || gltf.buffers.is_empty() {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!("bufferViews[{index}].buffer={buffer} 越界（只有 buffer 0）"),
                });
            }
            let Some(length) = view.byte_length else {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!("bufferViews[{index}] 缺少 byteLength"),
                });
            };
            let offset = view.byte_offset.unwrap_or(0);
            if length < 0 || offset < 0 {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!("bufferViews[{index}] 的 byteOffset/byteLength 非法（负值）"),
                });
            }
            let (offset, length) = (offset as u64, length as u64);
            let buffer_len = gltf.buffers[0].byte_length.unwrap_or(0) as u64;
            if offset + length > buffer_len {
                return Err(GlbError::Structure {
                    code: "gltf_buffer_range",
                    detail: format!(
                        "bufferViews[{index}]（offset={offset}, length={length}）超出 buffer 长度（{buffer_len} 字节）"
                    ),
                });
            }
            let stride = match view.byte_stride {
                None => None,
                Some(stride) if (4..=252).contains(&stride) && stride % 4 == 0 => {
                    Some(stride as u64)
                }
                Some(stride) => {
                    return Err(GlbError::Structure {
                        code: "gltf_buffer_range",
                        detail: format!(
                            "bufferViews[{index}].byteStride={stride} 非法（须为 4..=252 且 4 的倍数）"
                        ),
                    });
                }
            };
            views.push(BufferViewSpan {
                offset,
                length,
                stride,
            });
        }
        Ok(Self {
            bin_file_offset,
            bin_len,
            views,
            gltf,
        })
    }

    /// BIN chunk 中数据的绝对文件偏移（`offset` 相对 BIN 数据起点）。
    fn bin_absolute(&self, offset: u64) -> Result<u64, GlbError> {
        let start = self.bin_file_offset.ok_or_else(|| GlbError::Structure {
            code: "gltf_buffer_range",
            detail: "模型引用了 BIN 数据，但 GLB 没有 BIN chunk".to_owned(),
        })?;
        if offset > self.bin_len {
            return Err(GlbError::Structure {
                code: "gltf_buffer_range",
                detail: "访问超出 BIN chunk 的范围".to_owned(),
            });
        }
        start
            .checked_add(offset)
            .ok_or_else(|| GlbError::Structure {
                code: "gltf_buffer_range",
                detail: "偏移计算溢出".to_owned(),
            })
    }
}

/// 已验证的 accessor 布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccessorLayout {
    components: u64,
    /// 元素字节数（`component_size * components`）。
    element_size: u64,
    /// 元素间隔（bufferView.byteStride 或紧凑排列的 element_size）。
    stride: u64,
    count: u64,
    /// 第一个元素在 BIN 数据中的相对偏移。
    offset: u64,
    component_type: u64,
}

/// 校验单个 accessor 的范围（不读取数据）。
fn accessor_layout(
    layout: &Layout<'_>,
    index: i64,
    what: &str,
) -> Result<AccessorLayout, GlbError> {
    let accessors = &layout.gltf.accessors;
    let accessor = accessors
        .get(usize::try_from(index).unwrap_or(usize::MAX))
        .ok_or_else(|| GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!("{what} 引用的 accessor[{index}] 不存在"),
        })?;
    if accessor.sparse.is_some() {
        return Err(GlbError::Structure {
            code: "gltf_unsupported_feature",
            detail: format!("accessor[{index}] 使用 sparse：首版不支持 sparse accessor"),
        });
    }
    let count = accessor.count.unwrap_or(-1);
    if count < 0 {
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!("accessor[{index}] 缺少或非法的 count"),
        });
    }
    let count = count as u64;
    let component_type = accessor.component_type.unwrap_or(-1);
    let component_size = match component_type {
        5120 | 5121 => 1_u64,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        other => {
            return Err(GlbError::Structure {
                code: "gltf_accessor_type",
                detail: format!("accessor[{index}] 的 componentType={other} 不受支持"),
            });
        }
    };
    let components = match accessor.type_.as_deref() {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        Some("MAT2") => 4,
        Some("MAT3") => 9,
        Some("MAT4") => 16,
        other => {
            return Err(GlbError::Structure {
                code: "gltf_accessor_type",
                detail: format!(
                    "accessor[{index}] 的 type={} 不受支持",
                    other.unwrap_or("(缺失)")
                ),
            });
        }
    };
    let element_size = component_size * components;

    let Some(view_index) = accessor.buffer_view else {
        // 没有 bufferView 的 accessor 只能是全零初始化（glTF 允许）：没有数据可读，
        // 不能用于 POSITION/索引（调用方另行拒绝）。
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!("accessor[{index}] 没有 bufferView：该模型不是自包含几何数据"),
        });
    };
    let view_index = usize::try_from(view_index).map_err(|_| GlbError::Structure {
        code: "gltf_accessor_range",
        detail: format!("accessor[{index}].bufferView 非法"),
    })?;
    let view = layout
        .views
        .get(view_index)
        .ok_or_else(|| GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!("accessor[{index}] 引用的 bufferViews[{view_index}] 不存在"),
        })?;

    let byte_offset = accessor.byte_offset.unwrap_or(0);
    if byte_offset < 0 {
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!("accessor[{index}].byteOffset 非法（负值）"),
        });
    }
    let byte_offset = byte_offset as u64;
    let stride = view.stride.unwrap_or(element_size);
    if stride < element_size {
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!(
                "accessor[{index}] 的元素大小（{element_size} 字节）大于 bufferView 的 byteStride（{stride}）"
            ),
        });
    }
    // 对齐（glTF 规范：accessor 的字节偏移必须是 componentType 大小的整数倍）。
    if !byte_offset.is_multiple_of(component_size)
        || !(view.offset + byte_offset).is_multiple_of(component_size)
    {
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!(
                "accessor[{index}] 的字节偏移（{byte_offset}）未按 componentType 大小（{component_size}）对齐"
            ),
        });
    }
    // 范围：最后一个元素的最后一个分量必须落在 bufferView 之内。
    let span = if count == 0 {
        byte_offset
    } else {
        byte_offset
            .checked_add((count - 1).saturating_mul(stride))
            .and_then(|value| value.checked_add(element_size))
            .ok_or_else(|| GlbError::Structure {
                code: "gltf_accessor_range",
                detail: format!("accessor[{index}] 的范围计算溢出"),
            })?
    };
    if span > view.length {
        return Err(GlbError::Structure {
            code: "gltf_accessor_range",
            detail: format!(
                "accessor[{index}] 的数据范围（{span} 字节）超出 bufferViews[{view_index}] 的长度（{} 字节）\
                 （accessor 越界）",
                view.length
            ),
        });
    }

    Ok(AccessorLayout {
        components,
        element_size,
        stride,
        count,
        offset: view.offset + byte_offset,
        component_type: component_type as u64,
    })
}

// ---------------------------------------------------------------------------
// 几何（面数/顶点数/索引）
// ---------------------------------------------------------------------------

/// 一个已解析的 primitive。
#[derive(Debug, Clone, Copy)]
struct PrimitiveGeometry {
    position_accessor: usize,
    position_count: u64,
    indices: Option<usize>,
    triangles: u64,
}

fn collect_geometry(gltf: &Gltf, layout: &Layout<'_>) -> Result<Vec<PrimitiveGeometry>, GlbError> {
    let mut out = Vec::new();
    for (mesh_index, mesh) in gltf.meshes.iter().enumerate() {
        let Some(primitives) = &mesh.primitives else {
            continue;
        };
        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let what = format!("meshes[{mesh_index}].primitives[{primitive_index}]");
            let attributes = primitive
                .attributes
                .as_ref()
                .ok_or_else(|| GlbError::Geometry {
                    code: "gltf_missing_position",
                    detail: format!("{what} 缺少 attributes（无 POSITION，不是可绘制的几何）"),
                })?;
            let position_index =
                attributes
                    .get("POSITION")
                    .copied()
                    .ok_or_else(|| GlbError::Geometry {
                        code: "gltf_missing_position",
                        detail: format!("{what} 缺少 POSITION 属性（非空几何必须有顶点位置）"),
                    })?;
            let position = accessor_layout(layout, position_index, &format!("{what}.POSITION"))?;
            if position.component_type != 5126 || position.components != 3 {
                return Err(GlbError::Geometry {
                    code: "gltf_missing_position",
                    detail: format!("{what} 的 POSITION 必须是 FLOAT VEC3"),
                });
            }
            if position.count == 0 {
                return Err(GlbError::Geometry {
                    code: "gltf_empty_geometry",
                    detail: format!("{what} 的 POSITION 顶点数为 0（空几何）"),
                });
            }

            let mode = primitive.mode.unwrap_or(4);
            let (index_count, indices) = match primitive.indices {
                Some(index) => {
                    let accessor = accessor_layout(layout, index, &format!("{what}.indices"))?;
                    if accessor.components != 1
                        || !matches!(accessor.component_type, 5121 | 5123 | 5125)
                    {
                        return Err(GlbError::Geometry {
                            code: "gltf_index_out_of_range",
                            detail: format!(
                                "{what} 的索引 accessor 必须是 SCALAR 且 componentType ∈ \
                                 {{UNSIGNED_BYTE, UNSIGNED_SHORT, UNSIGNED_INT}}"
                            ),
                        });
                    }
                    (
                        accessor.count,
                        Some(usize::try_from(index).unwrap_or(usize::MAX)),
                    )
                }
                None => (position.count, None),
            };
            if !matches!(mode, 0..=6) {
                return Err(GlbError::Structure {
                    code: "gltf_unsupported_feature",
                    detail: format!("{what}.mode={mode} 不是合法的图元模式（0..=6）"),
                });
            }
            let triangles = match mode {
                4 => {
                    if index_count % 3 != 0 {
                        return Err(GlbError::Geometry {
                            code: "gltf_empty_geometry",
                            detail: format!(
                                "{what} 的顶点/索引数（{index_count}）不是 3 的整数倍（TRIANGLES）"
                            ),
                        });
                    }
                    index_count / 3
                }
                5 | 6 => index_count.saturating_sub(2),
                // POINTS/LINES、LINE_LOOP、LINE_STRIP：不产生三角面。
                _ => 0,
            };
            out.push(PrimitiveGeometry {
                position_accessor: usize::try_from(position_index).unwrap_or(usize::MAX),
                position_count: position.count,
                indices,
                triangles,
            });
        }
    }
    Ok(out)
}

fn check_budget(geometry: &[PrimitiveGeometry], budget: &GlbBudget) -> Result<(), GlbError> {
    let triangles: u64 = geometry.iter().map(|primitive| primitive.triangles).sum();
    if triangles == 0 {
        return Err(GlbError::Geometry {
            code: "gltf_empty_geometry",
            detail: "模型没有任何三角面（空几何）：不进入阅读器".to_owned(),
        });
    }
    if triangles > budget.max_triangles {
        return Err(GlbError::Budget {
            code: "gltf_face_limit",
            detail: format!(
                "三角面数（{triangles}）超过预算（{}）：请更换资料或重新生成（不自动降面数、不静默改坏模型）",
                budget.max_triangles
            ),
        });
    }
    Ok(())
}

/// 逐值检查 POSITION 是否有限，并累计整体包围盒。
fn check_positions<R: Read + Seek>(
    reader: &mut R,
    layout: &Layout<'_>,
    geometry: &[PrimitiveGeometry],
) -> Result<([f32; 3], [f32; 3]), GlbError> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut seen: Vec<usize> = Vec::new();
    for primitive in geometry {
        if seen.contains(&primitive.position_accessor) {
            continue;
        }
        seen.push(primitive.position_accessor);
        let accessor = accessor_layout(layout, primitive.position_accessor as i64, "POSITION")?;
        let start = layout.bin_absolute(accessor.offset)?;
        for_each_element(reader, start, &accessor, |element| {
            let values = [
                f32::from_le_bytes(element[0..4].try_into().expect("4 字节")),
                f32::from_le_bytes(element[4..8].try_into().expect("4 字节")),
                f32::from_le_bytes(element[8..12].try_into().expect("4 字节")),
            ];
            for (axis, value) in values.iter().enumerate() {
                if !value.is_finite() {
                    return Err(GlbError::Geometry {
                        code: "gltf_non_finite_position",
                        detail: format!(
                            "POSITION 含非有限坐标（NaN/Infinity，第 {} 轴）：模型数据无效，不进入阅读器",
                            axis + 1
                        ),
                    });
                }
                min[axis] = min[axis].min(*value);
                max[axis] = max[axis].max(*value);
            }
            Ok(())
        })?;
    }
    Ok((min, max))
}

/// 逐个索引检查 < 顶点数（读取 BIN；索引数量受面数预算与 bufferView 范围约束）。
fn check_indices<R: Read + Seek>(
    reader: &mut R,
    layout: &Layout<'_>,
    geometry: &[PrimitiveGeometry],
) -> Result<usize, GlbError> {
    let mut checked = 0_usize;
    for primitive in geometry {
        let Some(index) = primitive.indices else {
            continue;
        };
        let accessor = accessor_layout(layout, index as i64, "indices")?;
        let start = layout.bin_absolute(accessor.offset)?;
        let vertex_count = primitive.position_count;
        for_each_element(reader, start, &accessor, |element| {
            let value = match accessor.component_type {
                5121 => u64::from(element[0]),
                5123 => u64::from(u16::from_le_bytes([element[0], element[1]])),
                5125 => u64::from(u32::from_le_bytes(
                    element[0..4].try_into().expect("4 字节"),
                )),
                _ => unreachable!("componentType 已在 accessor_layout 校验"),
            };
            if value >= vertex_count {
                return Err(GlbError::Geometry {
                    code: "gltf_index_out_of_range",
                    detail: format!(
                        "索引值（{value}）超过对应 POSITION 的顶点数（{vertex_count}）：索引越界，模型数据无效"
                    ),
                });
            }
            Ok(())
        })?;
        checked += 1;
    }
    Ok(checked)
}

/// 按 accessor 布局分块遍历元素（每次最多约 1 MiB；不把整个 BIN 读进内存）。
fn for_each_element<R: Read + Seek>(
    reader: &mut R,
    absolute_start: u64,
    accessor: &AccessorLayout,
    mut visit: impl FnMut(&[u8]) -> Result<(), GlbError>,
) -> Result<(), GlbError> {
    const CHUNK_BYTES: u64 = 1 << 20;
    if accessor.count == 0 {
        return Ok(());
    }
    let per_chunk = (CHUNK_BYTES / accessor.stride).max(1);
    let mut index = 0_u64;
    let mut buffer: Vec<u8> = Vec::new();
    while index < accessor.count {
        let take = per_chunk.min(accessor.count - index);
        let span = (take - 1) * accessor.stride + accessor.element_size;
        buffer.resize(span as usize, 0);
        let start = absolute_start + index * accessor.stride;
        read_into(reader, start, &mut buffer)?;
        for element_index in 0..take {
            let offset = (element_index * accessor.stride) as usize;
            visit(&buffer[offset..offset + accessor.element_size as usize])?;
        }
        index += take;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 贴图（内嵌资源 + 尺寸预算）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct ImageInfo {
    max_dimension: u32,
}

/// 校验全部贴图：必须内嵌（bufferView）、必须 PNG/JPEG、单边 ≤ 预算。
fn check_images<R: Read + Seek>(
    reader: &mut R,
    gltf: &Gltf,
    layout: &Layout<'_>,
    budget: &GlbBudget,
) -> Result<Vec<ImageInfo>, GlbError> {
    let mut out = Vec::with_capacity(gltf.images.len());
    for (index, image) in gltf.images.iter().enumerate() {
        if image.uri.is_some() {
            return Err(GlbError::Structure {
                code: "gltf_external_uri",
                detail: format!(
                    "images[{index}] 带 uri（外部引用或 data: URI）：首版只接受 bufferView 内嵌的 PNG/JPEG 贴图"
                ),
            });
        }
        if let Some(mime) = &image.mime_type
            && !matches!(mime.as_str(), "image/png" | "image/jpeg")
        {
            return Err(GlbError::Structure {
                code: "gltf_unsupported_feature",
                detail: format!(
                    "images[{index}] 的 mimeType={mime} 不受支持（只接受内嵌 PNG/JPEG）"
                ),
            });
        }
        let Some(view_index) = image.buffer_view else {
            return Err(GlbError::Structure {
                code: "gltf_external_uri",
                detail: format!(
                    "images[{index}] 没有 bufferView（贴图未内嵌）：首版不接受外部贴图"
                ),
            });
        };
        let view_index = usize::try_from(view_index).map_err(|_| GlbError::Structure {
            code: "gltf_buffer_range",
            detail: format!("images[{index}].bufferView 非法"),
        })?;
        let view = layout
            .views
            .get(view_index)
            .ok_or_else(|| GlbError::Structure {
                code: "gltf_buffer_range",
                detail: format!("images[{index}] 引用的 bufferViews[{view_index}] 不存在"),
            })?;
        // 只读贴图头部即可判定格式与尺寸（PNG IHDR / JPEG SOF）。
        const HEADER_WINDOW: u64 = 64 * 1024;
        let window = view.length.min(HEADER_WINDOW) as usize;
        let start = layout.bin_absolute(view.offset)?;
        let bytes = read_exact_at(reader, start, window)?;
        let (width, height) = image_dimensions(&bytes).ok_or_else(|| GlbError::Structure {
            code: "gltf_image_invalid",
            detail: format!(
                "images[{index}] 不是可识别的内嵌 PNG/JPEG（尺寸不可判定）：贴图数据损坏或格式不支持"
            ),
        })?;
        if width == 0 || height == 0 {
            return Err(GlbError::Structure {
                code: "gltf_image_invalid",
                detail: format!("images[{index}] 的尺寸为 0（{width}×{height}）：贴图数据无效"),
            });
        }
        let max_dimension = width.max(height);
        if max_dimension > budget.max_texture_dimension {
            return Err(GlbError::Budget {
                code: "gltf_texture_limit",
                detail: format!(
                    "贴图尺寸（{width}×{height}）超过预算（单边上限 {} px）：请更换资料或重新生成\
                     （不自动压缩、不静默改坏模型）",
                    budget.max_texture_dimension
                ),
            });
        }
        out.push(ImageInfo { max_dimension });
    }
    Ok(out)
}

/// PNG/JPEG 头部解析（只读尺寸；不做像素解码）。
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return jpeg_dimensions(bytes);
    }
    None
}

/// JPEG：在 marker 链中找 SOF（尺寸段）。熵编码数据之前的段长度都可信。
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2_usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            return None;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        match marker {
            0xFF => {
                offset -= 1;
                continue;
            }
            // 无长度字段的 marker。
            0x01 | 0xD0..=0xD7 => continue,
            0xD8 => continue,
            0xD9 | 0xDA => return None, // EOI / SDS 之后没有 SOF：尺寸不可判定
            _ => {}
        }
        if offset + 2 > bytes.len() {
            return None;
        }
        let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if length < 2 || offset + length > bytes.len() {
            return None;
        }
        // SOF0..SOF15（除 DHT=D4、JPG=C4、DAC=CC 之外的 C0..CF）。
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]));
            return Some((width, height));
        }
        offset += length;
    }
    None
}

// ---------------------------------------------------------------------------
// 读取原语
// ---------------------------------------------------------------------------

fn read_exact_at<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, GlbError> {
    let mut buffer = vec![0_u8; len];
    read_into(reader, offset, &mut buffer)?;
    Ok(buffer)
}

fn read_into<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), GlbError> {
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|error| GlbError::Io {
            detail: error.to_string(),
        })?;
    reader.read_exact(buffer).map_err(|error| GlbError::Io {
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_defaults_match_the_contract() {
        let budget = GlbBudget::default();
        assert_eq!(budget.max_triangles, 100_000);
        assert_eq!(budget.max_texture_dimension, 4096);
    }

    #[test]
    fn error_codes_are_stable() {
        let error = GlbError::Budget {
            code: "gltf_face_limit",
            detail: "x".to_owned(),
        };
        assert_eq!(error.code(), "gltf_face_limit");
        assert!(error.is_budget());
        assert_eq!(
            GlbError::Container {
                code: "glb_magic",
                detail: "d".to_owned()
            }
            .code(),
            "glb_magic"
        );
    }

    #[test]
    fn image_dimensions_read_png_and_jpeg_headers() {
        // PNG：签名 + IHDR（16×16 之外的尺寸也可判定）。
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&8000_u32.to_be_bytes());
        png.extend_from_slice(&4096_u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((8000, 4096)));

        // JPEG：SOI + SOF0（段长 17、精度 8、高度 12、宽度 34、3 个分量）。
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0];
        jpeg.extend_from_slice(&17_u16.to_be_bytes());
        jpeg.push(8);
        jpeg.extend_from_slice(&12_u16.to_be_bytes());
        jpeg.extend_from_slice(&34_u16.to_be_bytes());
        jpeg.push(3);
        jpeg.extend_from_slice(&[0u8; 18]); // 分量定义 + 后续（段长包含自身 2 字节）
        assert_eq!(image_dimensions(&jpeg), Some((34, 12)));

        // 尺寸不可判定（没有 SOF）与未知格式都返回 None。
        let no_sof = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        assert_eq!(image_dimensions(&no_sof), None);
        assert_eq!(image_dimensions(b"RIFF....WEBP"), None);
    }
}
