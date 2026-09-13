//! QA 回合 15 · T13 独立验收用例（AC-043 / AC-044）。
//!
//! 独立性声明：本文件由 QA 现场编写，**不复用** `tests/model_assets.rs` 的用例或断言，
//! 也不调用 `test_support::generate::build_glb`／T05 场景脚本：
//! - GLB 负例的字节全部由 QA 手写（下表常量 + [`qa_assemble`]）；
//! - 下载负例使用 QA 手写的**原始 TCP fixture**（[`QaServer`]，只绑定 127.0.0.1），
//!   逐请求记录方法/target/全部请求头/连接数，可脚本化 302／半关闭／chunked／断连。
//!
//! 覆盖（逐条对应 REQ-028 / AC-043 / AC-044 与 architecture.md §7）：
//! - 容器：magic/version/声明长度/chunk 布局/未知 chunk/JSON 解析；
//! - 数据：bufferView/accessor 范围、对齐、byteStride、索引边界、有限坐标、非空几何；
//! - 资源与扩展：外链 buffer/image（含 `data:`）、required extension、sparse、贴图格式；
//! - 预算：面数、贴图单边（注入更小预算），失败后**原始文件字节不变**；
//! - 下载：连接 pin 到已验证 IP 且保留 Host 头、请求不带凭据（含"API client 带 bearer"对照）、
//!   302 → 私网/非允许域/文件协议、逐跳上限、DNS 重绑定与混合答案、明文与地址拒绝、
//!   声明长度与流中计数两道大小门、截断可安全重试（整文件重下，无 Range）、磁盘满。
//!
//! 零真实外网：全部 HTTP 目标为 127.0.0.1 上由本文件启动的 fixture。

mod common;

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::TestDir;
use everything_manual::assets::blob_store::SpaceProbe;
use everything_manual::assets::glb::{
    DownloadError, DownloadPolicy, GlbBudget, GlbError, HostResolver, ModelDownloader,
    ResolveFuture, inspect, inspect_glb_file,
};
use everything_manual::config::SecretString;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{ExecutorConfig, JobExecutor, ManualClock, StageRegistry};
use everything_manual::providers::tripo::TripoHandlers;
use everything_manual::providers::tripo::client::{TripoClient, TripoTimeouts};
use everything_manual::storage::repo::job_stages as stages_repo;
use manual_core::domain::{JobStage, JobStatus, StageKind};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_support::sha256_hex;

// ---------------------------------------------------------------------------
// A. QA 自建 GLB 字节（4 顶点 / 2 三角面 / 内嵌 8×4 PNG）
// ---------------------------------------------------------------------------

const QA_POS_OFFSET: usize = 0;
const QA_POS_LEN: usize = 48;
const QA_IDX_OFFSET: usize = 48;
const QA_IDX_LEN: usize = 12;
const QA_PNG_OFFSET: usize = 60;

/// 结构正确的 PNG IHDR 头（33 字节；校验器只读尺寸，不解码像素）。
fn qa_png_ihdr(width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&[0, 0, 0, 0]);
    png
}

fn qa_bin(width: u32, height: u32) -> Vec<u8> {
    let mut bin = Vec::new();
    let vertices: [[f32; 3]; 4] = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    for vertex in vertices {
        for axis in vertex {
            bin.extend_from_slice(&axis.to_le_bytes());
        }
    }
    assert_eq!(bin.len(), QA_IDX_OFFSET);
    for index in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    assert_eq!(bin.len(), QA_PNG_OFFSET);
    bin.extend_from_slice(&qa_png_ihdr(width, height));
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    bin
}

fn qa_json(bin_len: usize, png_len: usize) -> Value {
    json!({
        "asset": { "version": "2.0" },
        "buffers": [ { "byteLength": bin_len } ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": QA_POS_OFFSET, "byteLength": QA_POS_LEN },
            { "buffer": 0, "byteOffset": QA_IDX_OFFSET, "byteLength": QA_IDX_LEN },
            { "buffer": 0, "byteOffset": QA_PNG_OFFSET, "byteLength": png_len },
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3" },
            { "bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR" },
        ],
        "meshes": [ { "primitives": [
            { "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4 }
        ] } ],
        "images": [ { "bufferView": 2, "mimeType": "image/png" } ],
    })
}

fn qa_valid_bin() -> Vec<u8> {
    qa_bin(8, 4)
}

fn qa_valid_json() -> Value {
    let bin = qa_valid_bin();
    qa_json(bin.len(), qa_png_ihdr(8, 4).len())
}

fn qa_valid_bytes() -> Vec<u8> {
    qa_assemble(&qa_valid_json(), Some(&qa_valid_bin()))
}

/// 组装 GLB（JSON 补齐 4 字节、BIN 可选）；头内声明长度 = 实际字节数。
fn qa_assemble(json_value: &Value, bin: Option<&[u8]>) -> Vec<u8> {
    let mut json_bytes = serde_json::to_vec(json_value).expect("序列化 JSON");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin_bytes = bin.map(<[u8]>::to_vec).unwrap_or_default();
    if !bin_bytes.is_empty() {
        while !bin_bytes.len().is_multiple_of(4) {
            bin_bytes.push(0);
        }
    }
    let total = 12
        + 8
        + json_bytes.len()
        + if bin_bytes.is_empty() {
            0
        } else {
            8 + bin_bytes.len()
        };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    if !bin_bytes.is_empty() {
        out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin_bytes);
    }
    out
}

fn qa_inspect(
    bytes: &[u8],
    budget: &GlbBudget,
) -> Result<everything_manual::assets::glb::GlbSummary, GlbError> {
    let mut cursor = std::io::Cursor::new(bytes.to_vec());
    inspect(&mut cursor, bytes.len() as u64, budget)
}

fn qa_code(bytes: &[u8], budget: &GlbBudget) -> String {
    match qa_inspect(bytes, budget) {
        Ok(summary) => panic!("本应被拒绝，却通过：{summary:?}"),
        Err(error) => error.code().to_owned(),
    }
}

/// 以有效 JSON 为底修改后重建（BIN 不变；`byteLength` 不随修改重算——修改者自行负责）。
fn qa_bytes_with(mutator: impl FnOnce(&mut Value)) -> Vec<u8> {
    let bin = qa_valid_bin();
    let mut json = qa_valid_json();
    mutator(&mut json);
    qa_assemble(&json, Some(&bin))
}

// ---------------------------------------------------------------------------
// A1. 容器负例（手写字节）
// ---------------------------------------------------------------------------

#[test]
fn qa_container_negatives_reject_handcrafted_bytes() {
    // 正常基线：QA 自建字节必须先通过（否则负例无意义）。
    let good = qa_valid_bytes();
    let summary = qa_inspect(&good, &GlbBudget::default()).expect("QA 自建样例应通过");
    assert_eq!(summary.triangles, 2);
    assert_eq!(summary.vertices, 4);
    assert_eq!(summary.max_texture_dimension, 8);

    // magic。
    let mut bad = good.clone();
    bad[0..4].copy_from_slice(b"GLTF");
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_magic");

    // version=1 / version=3。
    for version in [1u32, 3] {
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&version.to_le_bytes());
        assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_version");
    }

    // 声明长度大于实际（截断）与小于实际（多出数据）都必须拒绝。
    for delta in [4i64, -4] {
        let mut bad = good.clone();
        let declared = u32::from_le_bytes(bad[8..12].try_into().unwrap()) as i64;
        bad[8..12].copy_from_slice(&((declared + delta) as u32).to_le_bytes());
        assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_declared_length");
    }

    // 头不完整（12 字节以内）。
    assert_eq!(qa_code(&good[..10], &GlbBudget::default()), "glb_magic");

    // JSON chunk 声明长度不是 4 的倍数。
    let mut bad = good.clone();
    let json_len = u32::from_le_bytes(bad[12..16].try_into().unwrap());
    bad[12..16].copy_from_slice(&(json_len - 1).to_le_bytes());
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_chunk_layout");

    // JSON chunk 长度为 0。
    let mut bad = good.clone();
    bad[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_chunk_layout");

    // 首 chunk 不是 JSON。
    let mut bad = good.clone();
    bad[16..20].copy_from_slice(b"BIN\0");
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_chunk_layout");

    // 第三个 chunk（未知类型）必须拒绝。
    let mut bad = good.clone();
    let extra_body = [0u8; 4];
    bad.extend_from_slice(&(extra_body.len() as u32).to_le_bytes());
    bad.extend_from_slice(b"EXT\0");
    bad.extend_from_slice(&extra_body);
    let total = bad.len() as u32;
    bad[8..12].copy_from_slice(&total.to_le_bytes());
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_chunk_layout");

    // JSON 不可解析。
    let mut bad = good.clone();
    let json_len = u32::from_le_bytes(bad[12..16].try_into().unwrap()) as usize;
    bad[20..20 + json_len].fill(b'x');
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_json");

    // JSON 不是对象（数组）。
    let mut bad = qa_assemble(&json!([1, 2, 3]), None);
    let declared = bad.len() as u32;
    bad[8..12].copy_from_slice(&declared.to_le_bytes());
    assert_eq!(qa_code(&bad, &GlbBudget::default()), "glb_json");
}

// ---------------------------------------------------------------------------
// A2. buffer / bufferView / accessor 负例
// ---------------------------------------------------------------------------

#[test]
fn qa_layout_negatives_reject_out_of_range_and_alignment() {
    // accessor.count 超出 bufferView。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["count"] = json!(999)),
            &GlbBudget::default()
        ),
        "gltf_accessor_range"
    );

    // accessor.byteOffset 未按 componentType 对齐（FLOAT 需 4 的倍数）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["byteOffset"] = json!(2)),
            &GlbBudget::default()
        ),
        "gltf_accessor_range"
    );

    // byteStride 非法（3 不是 4 的倍数）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["bufferViews"][0]["byteStride"] = json!(3)),
            &GlbBudget::default()
        ),
        "gltf_buffer_range"
    );

    // byteStride 小于元素大小（VEC3 FLOAT = 12 字节 → stride=4 必须拒绝）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["bufferViews"][0]["byteStride"] = json!(4)),
            &GlbBudget::default()
        ),
        "gltf_accessor_range"
    );

    // bufferView 超出 buffer 声明长度。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["bufferViews"][1]["byteLength"] = json!(4096)),
            &GlbBudget::default()
        ),
        "gltf_buffer_range"
    );

    // buffer.byteLength 超过 BIN chunk 实际长度。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["buffers"][0]["byteLength"] = json!(4096)),
            &GlbBudget::default()
        ),
        "gltf_buffer_range"
    );

    // 第二个 buffer 条目（GLB 只允许 buffer 0）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["buffers"] = json!([{ "byteLength": 96 }, { "byteLength": 4 }]);
            }),
            &GlbBudget::default()
        ),
        "gltf_buffer_range"
    );

    // bufferView 引用不存在的 buffer 1。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["buffers"] = json!([{ "byteLength": 96 }, { "byteLength": 96 }]);
                json["bufferViews"][0]["buffer"] = json!(1);
            }),
            &GlbBudget::default()
        ),
        "gltf_buffer_range"
    );

    // bufferView 引用不存在的索引。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["bufferView"] = json!(9)),
            &GlbBudget::default()
        ),
        "gltf_accessor_range"
    );

    // accessor 没有 bufferView。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["accessors"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("bufferView");
            }),
            &GlbBudget::default()
        ),
        "gltf_accessor_range"
    );

    // componentType 不受支持（5127 非法）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["componentType"] = json!(5127)),
            &GlbBudget::default()
        ),
        "gltf_accessor_type"
    );

    // type 不受支持。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["type"] = json!("VEC5")),
            &GlbBudget::default()
        ),
        "gltf_accessor_type"
    );

    // sparse accessor：明确拒绝。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["accessors"][0]["sparse"] = json!({ "count": 1, "indices": {}, "values": {} });
            }),
            &GlbBudget::default()
        ),
        "gltf_unsupported_feature"
    );

    // 引用 BIN 数据但没有 BIN chunk。
    let json = qa_valid_json();
    assert_eq!(
        qa_code(&qa_assemble(&json, None), &GlbBudget::default()),
        "gltf_buffer_range"
    );
}

// ---------------------------------------------------------------------------
// A3. 几何负例（含 BIN 字节篡改：非有限坐标、索引越界）
// ---------------------------------------------------------------------------

#[test]
fn qa_geometry_negatives_reject_bad_data() {
    // 缺少 POSITION。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["meshes"][0]["primitives"][0]["attributes"] = json!({});
            }),
            &GlbBudget::default()
        ),
        "gltf_missing_position"
    );

    // POSITION 不是 FLOAT VEC3（改成 VEC2）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["type"] = json!("VEC2")),
            &GlbBudget::default()
        ),
        "gltf_missing_position"
    );

    // POSITION count=0（空几何）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][0]["count"] = json!(0)),
            &GlbBudget::default()
        ),
        "gltf_empty_geometry"
    );

    // 无索引且顶点数不是 3 的倍数 → 不能构成三角面。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| {
                json["meshes"][0]["primitives"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("indices");
            }),
            &GlbBudget::default()
        ),
        "gltf_empty_geometry"
    );

    // 线框（mode=1）不产生三角面。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["meshes"][0]["primitives"][0]["mode"] = json!(1)),
            &GlbBudget::default()
        ),
        "gltf_empty_geometry"
    );

    // 非法 mode。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["meshes"][0]["primitives"][0]["mode"] = json!(7)),
            &GlbBudget::default()
        ),
        "gltf_unsupported_feature"
    );

    // 没有 mesh（空几何）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["meshes"] = json!([])),
            &GlbBudget::default()
        ),
        "gltf_empty_geometry"
    );

    // 索引 componentType 必须是 UBYTE/USHORT/UINT（BYTE=5120 拒绝）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["accessors"][1]["componentType"] = json!(5120)),
            &GlbBudget::default()
        ),
        "gltf_index_out_of_range"
    );

    // NaN 坐标（Q_NaN = 0x7FC00000）。
    let mut bin = qa_valid_bin();
    bin[0..4].copy_from_slice(&0x7FC0_0000u32.to_le_bytes());
    assert_eq!(
        qa_code(
            &qa_assemble(&qa_valid_json(), Some(&bin)),
            &GlbBudget::default()
        ),
        "gltf_non_finite_position"
    );

    // +Infinity 坐标（0x7F800000）。
    let mut bin = qa_valid_bin();
    bin[8..12].copy_from_slice(&0x7F80_0000u32.to_le_bytes());
    let error = qa_inspect(
        &qa_assemble(&qa_valid_json(), Some(&bin)),
        &GlbBudget::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "gltf_non_finite_position");
    assert!(error.message().contains("NaN") || error.message().contains("Infinity"));

    // 索引越界（顶点数 4 → 索引 9 非法）。
    let mut bin = qa_valid_bin();
    bin[QA_IDX_OFFSET..QA_IDX_OFFSET + 2].copy_from_slice(&9u16.to_le_bytes());
    let error = qa_inspect(
        &qa_assemble(&qa_valid_json(), Some(&bin)),
        &GlbBudget::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "gltf_index_out_of_range");
}

// ---------------------------------------------------------------------------
// A4. 内嵌资源 / 扩展 / 贴图负例
// ---------------------------------------------------------------------------

#[test]
fn qa_resource_and_extension_negatives() {
    // buffer 外链（http 与 data: 都拒绝）。
    for uri in [
        "https://cdn.evil.invalid/model.bin",
        "data:application/octet-stream;base64,AAAAAA==",
    ] {
        let mut json = qa_valid_json();
        json["buffers"][0]["uri"] = json!(uri);
        let error = qa_inspect(
            &qa_assemble(&json, Some(&qa_valid_bin())),
            &GlbBudget::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "gltf_external_uri", "{uri}");
    }

    // image 外链（含 data: base64 贴图）。
    for uri in [
        "https://cdn.evil.invalid/tex.png",
        "data:image/png;base64,iVBORw0KGgo=",
    ] {
        let mut json = qa_valid_json();
        json["images"][0] = json!({ "uri": uri, "mimeType": "image/png" });
        let error = qa_inspect(
            &qa_assemble(&json, Some(&qa_valid_bin())),
            &GlbBudget::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "gltf_external_uri", "{uri}");
    }

    // image 既没有 uri 也没有 bufferView。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["images"][0] = json!({ "mimeType": "image/png" })),
            &GlbBudget::default()
        ),
        "gltf_external_uri"
    );

    // 贴图 MIME 不受支持（webp）。
    assert_eq!(
        qa_code(
            &qa_bytes_with(|json| json["images"][0]["mimeType"] = json!("image/webp")),
            &GlbBudget::default()
        ),
        "gltf_unsupported_feature"
    );

    // 贴图数据不是 PNG/JPEG（bufferView 内容为随机字节）。
    let mut bin = qa_valid_bin();
    let png_len = qa_png_ihdr(8, 4).len();
    bin[QA_PNG_OFFSET..QA_PNG_OFFSET + png_len].fill(0x5A);
    assert_eq!(
        qa_code(
            &qa_assemble(&qa_valid_json(), Some(&bin)),
            &GlbBudget::default()
        ),
        "gltf_image_invalid"
    );

    // PNG 尺寸为 0。
    let bad_png = qa_png_ihdr(0, 4);
    let mut bin = qa_valid_bin();
    bin[QA_PNG_OFFSET..QA_PNG_OFFSET + bad_png.len()].copy_from_slice(&bad_png);
    assert_eq!(
        qa_code(
            &qa_assemble(&qa_valid_json(), Some(&bin)),
            &GlbBudget::default()
        ),
        "gltf_image_invalid"
    );

    // required extension（Draco）。
    let mut json = qa_valid_json();
    json["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    let error = qa_inspect(
        &qa_assemble(&json, Some(&qa_valid_bin())),
        &GlbBudget::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "gltf_required_extension");
    assert!(error.message().contains("KHR_draco_mesh_compression"));

    // extensionsRequired 为空数组不构成拒绝（正常样例本身就带空/缺失）。
    let mut json = qa_valid_json();
    json["extensionsRequired"] = json!([]);
    assert!(
        qa_inspect(
            &qa_assemble(&json, Some(&qa_valid_bin())),
            &GlbBudget::default()
        )
        .is_ok()
    );

    // asset.version 必须是 "2.0"。
    for version in ["1.0", "2.1", ""] {
        assert_eq!(
            qa_code(
                &qa_bytes_with(|json| json["asset"]["version"] = json!(version)),
                &GlbBudget::default()
            ),
            "gltf_asset_version"
        );
    }
}

// ---------------------------------------------------------------------------
// A5. 预算负例 + 失败后原始文件字节不变
// ---------------------------------------------------------------------------

#[test]
fn qa_budget_negatives_and_original_file_is_untouched() {
    let dir = TestDir::new("qa13-budget");
    let path = dir.join("qa-model.glb");
    let bytes = qa_valid_bytes();
    std::fs::write(&path, &bytes).expect("写入 QA 模型");
    let before = sha256_hex(&bytes);

    // 面数超预算（2 面 > 1）。
    let error = inspect_glb_file(
        &path,
        &GlbBudget {
            max_triangles: 1,
            max_texture_dimension: 4096,
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "gltf_face_limit");
    assert!(
        error.is_budget(),
        "面数超限必须分类为预算（needs_input 提示不自动降预算）"
    );
    assert!(error.message().contains("不自动降面数"), "{error:?}");

    // 贴图超预算（8×4 > 单边 4）。
    let error = inspect_glb_file(
        &path,
        &GlbBudget {
            max_triangles: 100_000,
            max_texture_dimension: 4,
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "gltf_texture_limit");
    assert!(error.is_budget());

    // 失败不得修改原始模型字节（不静默改坏模型）。
    let after_bytes = std::fs::read(&path).expect("重读 QA 模型");
    assert_eq!(
        sha256_hex(&after_bytes),
        before,
        "校验失败后文件字节必须不变"
    );
    assert_eq!(after_bytes, bytes);

    // 默认预算（100000 面 / 4096 px）下同一文件通过。
    let summary = inspect_glb_file(&path, &GlbBudget::default()).expect("默认预算应通过");
    assert_eq!(summary.triangles, 2);
    assert_eq!(summary.images, 1);
    assert!(summary.bounds_min[0] <= -1.0 && summary.bounds_max[0] >= 1.0);
}

// ---------------------------------------------------------------------------
// B. QA 自建原始 TCP fixture（记录请求头/连接数；只绑定 127.0.0.1）
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum QaCanned {
    /// 立即响应；若 `headers` 里带 `content-length`，按该声明值写出（可与 body 长度不同）。
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// `transfer-encoding: chunked` 流式响应（无 Content-Length）。
    Chunked {
        status: u16,
        body: Vec<u8>,
        chunk: usize,
    },
    /// 写完整头与 `truncate` 字节 body 后关闭连接（半关闭截断）。
    HalfClose {
        status: u16,
        body: Vec<u8>,
        truncate: usize,
    },
    /// 不发任何字节直接关闭连接。
    Disconnect,
}

#[derive(Clone, Debug)]
struct QaRule {
    method: String,
    path: String,
    prefix: bool,
    repeat_last: bool,
    steps: Vec<QaCanned>,
}

#[derive(Clone, Debug)]
struct QaRequest {
    seq: usize,
    method: String,
    target: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl QaRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn header_names(&self) -> Vec<String> {
        self.headers
            .iter()
            .map(|(key, _)| key.to_ascii_lowercase())
            .collect()
    }
}

struct QaShared {
    rules: Vec<QaRule>,
    consumed: Mutex<Vec<usize>>,
    requests: Mutex<Vec<QaRequest>>,
    connections: AtomicUsize,
}

struct QaServer {
    addr: SocketAddr,
    shared: Arc<QaShared>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl QaServer {
    fn start(rules: Vec<QaRule>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 QA fixture 端口");
        listener.set_nonblocking(true).expect("非阻塞监听");
        let addr = listener.local_addr().expect("本地地址");
        let shared = Arc::new(QaShared {
            consumed: Mutex::new(vec![0; rules.len()]),
            rules,
            requests: Mutex::new(Vec::new()),
            connections: AtomicUsize::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread_shared.connections.fetch_add(1, Ordering::SeqCst);
                        let _ = handle_connection(stream, &thread_shared);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            shared,
            stop,
            thread: Some(thread),
        }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port(), path)
    }

    fn requests(&self) -> Vec<QaRequest> {
        self.shared.requests.lock().expect("请求记录").clone()
    }

    fn count(&self, method: &str, path: &str) -> usize {
        self.requests()
            .into_iter()
            .filter(|request| request.method.eq_ignore_ascii_case(method) && request.path == path)
            .count()
    }

    fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    fn host_header_for(&self, host: &str) -> String {
        format!("{host}:{}", self.port())
    }
}

impl Drop for QaServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn qa_respond(status: u16, headers: &[(&str, &str)], body: &[u8]) -> QaCanned {
    QaCanned::Respond {
        status,
        headers: headers
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.to_vec(),
    }
}

fn qa_json_respond(value: &Value) -> QaCanned {
    qa_respond(
        200,
        &[("content-type", "application/json")],
        &serde_json::to_vec(value).expect("JSON"),
    )
}

fn qa_rule(method: &str, path: &str, steps: Vec<QaCanned>) -> QaRule {
    QaRule {
        method: method.to_owned(),
        path: path.to_owned(),
        prefix: false,
        repeat_last: false,
        steps,
    }
}

fn qa_prefix_rule(method: &str, path: &str, steps: Vec<QaCanned>) -> QaRule {
    QaRule {
        method: method.to_owned(),
        path: path.to_owned(),
        prefix: true,
        repeat_last: false,
        steps,
    }
}

/// 同一步骤重复响应（自循环重定向、重复下载等）。
fn qa_repeat_rule(method: &str, path: &str, steps: Vec<QaCanned>) -> QaRule {
    QaRule {
        method: method.to_owned(),
        path: path.to_owned(),
        prefix: false,
        repeat_last: true,
        steps,
    }
}

fn handle_connection(mut stream: TcpStream, shared: &QaShared) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 4096];
    let header_end = loop {
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
        match stream.read(&mut scratch) {
            Ok(0) => return Ok(()),
            Ok(read) => buffer.extend_from_slice(&scratch[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return Ok(()),
            Err(error) => return Err(error),
        }
        if buffer.len() > 4 * 1024 * 1024 {
            return Ok(());
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end.0]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let declared: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    let mut missing = declared.saturating_sub(buffer.len() - header_end.1);
    while missing > 0 {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(read) => missing = missing.saturating_sub(read),
            Err(_) => break,
        }
    }

    let path = target.split('?').next().unwrap_or_default().to_owned();
    let sequence = {
        let mut requests = shared.requests.lock().expect("请求记录");
        let sequence = requests.len();
        requests.push(QaRequest {
            seq: sequence,
            method: method.clone(),
            target: target.clone(),
            path: path.clone(),
            headers: headers.clone(),
        });
        sequence
    };
    let _ = sequence;

    let canned = shared
        .rules
        .iter()
        .enumerate()
        .find(|(_, rule)| {
            rule.method.eq_ignore_ascii_case(&method)
                && if rule.prefix {
                    path.starts_with(&rule.path)
                } else {
                    path == rule.path
                }
        })
        .and_then(|(index, rule)| {
            let mut consumed = shared.consumed.lock().expect("步骤计数");
            let taken = consumed[index];
            if taken >= rule.steps.len() && !rule.repeat_last {
                return None;
            }
            let step = rule.steps[taken.min(rule.steps.len() - 1)].clone();
            consumed[index] = taken + 1;
            Some(step)
        });

    match canned {
        Some(QaCanned::Respond {
            status,
            headers,
            body,
        }) => {
            let declared_length = headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(body.len());
            let mut out = format!("HTTP/1.1 {status} {}\r\n", qa_reason(status));
            for (key, value) in &headers {
                out.push_str(&format!("{key}: {value}\r\n"));
            }
            out.push_str(&format!("content-length: {declared_length}\r\n"));
            out.push_str("connection: close\r\n\r\n");
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
        Some(QaCanned::Chunked {
            status,
            body,
            chunk,
        }) => {
            let mut out = format!("HTTP/1.1 {status} {}\r\n", qa_reason(status));
            out.push_str("content-type: application/octet-stream\r\n");
            out.push_str("transfer-encoding: chunked\r\n");
            out.push_str("connection: close\r\n\r\n");
            let _ = stream.write_all(out.as_bytes());
            for piece in body.chunks(chunk.max(1)) {
                let header = format!("{:x}\r\n", piece.len());
                if stream.write_all(header.as_bytes()).is_err()
                    || stream.write_all(piece).is_err()
                    || stream.write_all(b"\r\n").is_err()
                {
                    break;
                }
                let _ = stream.flush();
            }
            let _ = stream.write_all(b"0\r\n\r\n");
            let _ = stream.flush();
        }
        Some(QaCanned::HalfClose {
            status,
            body,
            truncate,
        }) => {
            let mut out = format!("HTTP/1.1 {status} {}\r\n", qa_reason(status));
            out.push_str("content-type: application/octet-stream\r\n");
            out.push_str(&format!("content-length: {}\r\n", body.len()));
            out.push_str("connection: close\r\n\r\n");
            let _ = stream.write_all(out.as_bytes());
            let truncated = truncate.min(body.len());
            let _ = stream.write_all(&body[..truncated]);
            let _ = stream.flush();
            let _ = stream.shutdown(Shutdown::Both);
        }
        Some(QaCanned::Disconnect) | None => {}
    }
    Ok(())
}

/// 返回 `(头结束位置, 头+CRLFCRLF 的总长度)`。
fn find_header_end(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, position + 4))
}

fn qa_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn qa_policy(hosts: &[&str], max_bytes: u64) -> DownloadPolicy {
    DownloadPolicy {
        allowed_hosts: hosts.iter().map(|host| (*host).to_owned()).collect(),
        allow_local_fixture: true,
        max_bytes,
        max_redirects: 5,
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(30),
    }
}

/// QA 自建静态解析器（模拟 DNS；用于重绑定/混合答案）。
struct QaResolver {
    entries: Vec<(String, Vec<std::net::IpAddr>)>,
}

impl QaResolver {
    fn new(entries: &[(&str, &[&str])]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(host, addresses)| {
                    (
                        (*host).to_owned(),
                        addresses
                            .iter()
                            .map(|ip| ip.parse().expect("测试 IP"))
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl HostResolver for QaResolver {
    fn resolve<'a>(&'a self, host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let result = self
            .entries
            .iter()
            .find(|(name, _)| name == host)
            .map(|(_, addresses)| addresses.clone())
            .ok_or_else(|| DownloadError::ResolutionFailed {
                host: host.to_owned(),
                detail: "QA 解析器没有该主机".to_owned(),
            });
        Box::pin(std::future::ready(result))
    }
}

fn qa_download_dir(tag: &str) -> TestDir {
    TestDir::new(tag)
}

fn qa_tmp_parts(data_dir: &Path) -> Vec<String> {
    let tmp = data_dir.join("tmp");
    std::fs::read_dir(&tmp)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".part"))
                .collect()
        })
        .unwrap_or_default()
}

fn qa_blob_count(data_dir: &Path) -> usize {
    let mut count = 0;
    let Ok(prefixes) = std::fs::read_dir(data_dir.join("blobs")) else {
        return 0;
    };
    for prefix in prefixes.filter_map(Result::ok) {
        if let Ok(files) = std::fs::read_dir(prefix.path()) {
            count += files.filter_map(Result::ok).count();
        }
    }
    count
}

// ---------------------------------------------------------------------------
// B1. 连接 pin 到已验证 IP + 保留 hostname（Host 头）+ 不带任何凭据
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_pin_ip_keeps_hostname_and_sends_no_credentials() {
    let payload = qa_valid_bytes();
    let expected_sha = sha256_hex(&payload);
    let payload_for_server = payload.clone();
    let server = QaServer::start(vec![
        qa_prefix_rule(
            "GET",
            "/model.glb",
            vec![QaCanned::Respond {
                status: 200,
                headers: vec![("content-type".to_owned(), "model/gltf-binary".to_owned())],
                body: payload_for_server,
            }],
        ),
        qa_rule(
            "GET",
            "/v3/tasks/qa13-task",
            vec![qa_json_respond(&json!({
                "code": 0,
                "data": { "task_id": "qa13-task", "status": "success", "progress": 100 }
            }))],
        ),
    ]);

    // 允许域名解析到回环（注入解析器）：若实现没有 pin 已校验 IP 而是再次用系统 DNS，
    // 该域名（.test 保留域）不可能解析成功——成功连接本身就证明连接使用了验证过的 IP。
    let dir = qa_download_dir("qa13-pin");
    let downloader =
        ModelDownloader::new(dir.path(), qa_policy(&["pin.qa13.test"], 1_048_576)).with_resolver(
            Arc::new(QaResolver::new(&[("pin.qa13.test", &["127.0.0.1"])])),
        );

    // URL 使用允许域名（而非 IP 字面量）：连接 pin 到解析出来的 127.0.0.1，
    // 但 Host 头与（真实场景中的）TLS SNI 必须是原域名。
    let url = format!(
        "http://pin.qa13.test:{}/model.glb?sign=qa13-canary-signature",
        server.port()
    );
    let downloaded = downloader
        .download(&url)
        .await
        .expect("pin 到已校验 IP 应成功");
    assert_eq!(downloaded.sha256, expected_sha);
    assert_eq!(downloaded.size, payload.len() as u64);

    let requests = server.requests();
    assert_eq!(requests.len(), 1, "合法下载只允许 1 个请求");
    let request = &requests[0];
    assert_eq!(request.seq, 0, "fixture 按到达顺序记录");
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/model.glb?sign=qa13-canary-signature");
    assert_eq!(
        request.header("host"),
        Some(server.host_header_for("pin.qa13.test").as_str()),
        "Host 头必须保持原 hostname（TLS SNI 同源）"
    );
    for forbidden in [
        "authorization",
        "proxy-authorization",
        "cookie",
        "x-api-key",
        "api-key",
        "x-auth-token",
        "range",
    ] {
        assert!(
            request.header(forbidden).is_none(),
            "模型 CDN 请求不得带 {forbidden}：{:?}",
            request.header_names()
        );
    }

    // 对照：同一 fixture 上，T12 的 API client（带假凭据）确实会发送 Authorization。
    let client = TripoClient::new(
        &format!("http://127.0.0.1:{}/v3", server.port()),
        SecretString::new("qa13-canary-api-key"),
        TripoTimeouts::default(),
    )
    .expect("构造 API client");
    let _ = client.get_task("qa13-task").await.expect("任务查询成功");
    let api_request = server
        .requests()
        .into_iter()
        .find(|request| request.path == "/v3/tasks/qa13-task")
        .expect("API 请求已记录");
    assert_eq!(
        api_request.header("authorization"),
        Some("Bearer qa13-canary-api-key"),
        "对照：API client 必须携带 bearer（证明凭据只在 API 客户端上）"
    );
}

// ---------------------------------------------------------------------------
// B2. 重定向负例：逐跳校验（私网/非允许域/协议/无 Location/3xx 未识别）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_redirect_negatives_never_reach_the_next_hop() {
    let second = QaServer::start(vec![qa_prefix_rule(
        "GET",
        "/x.glb",
        vec![qa_respond(200, &[], b"SHOULD-NEVER-BE-SERVED")],
    )]);

    let server = QaServer::start(vec![
        qa_rule(
            "GET",
            "/r-private",
            vec![qa_respond(
                302,
                &[("location", "http://10.1.2.3/x.glb")],
                b"",
            )],
        ),
        qa_rule(
            "GET",
            "/r-notallowed",
            vec![qa_respond(
                302,
                &[(
                    "location",
                    &format!("http://not-allowed.qa13.test:{}/x.glb", second.port()),
                )],
                b"",
            )],
        ),
        qa_rule(
            "GET",
            "/r-ftp",
            vec![qa_respond(
                302,
                &[("location", "ftp://evil.qa13.test/x.glb")],
                b"",
            )],
        ),
        qa_rule(
            "GET",
            "/r-file",
            vec![qa_respond(302, &[("location", "file:///etc/passwd")], b"")],
        ),
        qa_rule("GET", "/r-nolocation", vec![qa_respond(302, &[], b"")]),
        qa_rule(
            "GET",
            "/r-300",
            vec![qa_respond(300, &[("location", &second.url("/x.glb"))], b"")],
        ),
        qa_repeat_rule(
            "GET",
            "/r-loop",
            vec![qa_respond(302, &[("location", "/r-loop")], b"")],
        ),
    ]);

    let dir = qa_download_dir("qa13-redirect");
    let data_dir = dir.path();

    // 1) 302 → 私网 IP 字面量（即使允许域名单里含该 IP，地址校验也必须拒绝）。
    let downloader =
        ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1", "10.1.2.3"], 1_048_576));
    let error = downloader
        .download(&server.url("/r-private"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address", "{error:?}");
    assert!(error.message().contains("私网地址"), "{error:?}");
    assert_eq!(server.count("GET", "/r-private"), 1, "只允许第 1 跳");
    assert_eq!(second.connections(), 0, "第 2 跳不得建立连接");

    // 2) 302 → 不在允许域名单内的主机（指向本机第二个 fixture）。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1"], 1_048_576));
    let error = downloader
        .download(&server.url("/r-notallowed"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_host_not_allowed", "{error:?}");
    assert_eq!(server.count("GET", "/r-notallowed"), 1);
    assert_eq!(second.connections(), 0, "未允许域不得被连接");

    // 3) 302 → 非 http(s) 协议（ftp）。
    let error = downloader
        .download(&server.url("/r-ftp"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_insecure_scheme", "{error:?}");

    // 3b) 302 → file:// （无主机名；fail-closed：InvalidUrl 或 scheme 拒绝都可接受，
    //     关键是不读取本地文件、不产生资产）。QA 实测为 `download_invalid_url`。
    let error = downloader
        .download(&server.url("/r-file"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error.code(),
            "download_invalid_url" | "download_insecure_scheme"
        ),
        "file:// 重定向必须被拒绝，实际 {error:?}"
    );

    // 4) 302 但没有 Location。
    let error = downloader
        .download(&server.url("/r-nolocation"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_bad_redirect", "{error:?}");

    // 5) 300（未识别的 3xx）必须明确失败，不跟随。
    let error = downloader
        .download(&server.url("/r-300"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_http_status", "{error:?}");
    assert_eq!(second.connections(), 0);

    // 6) 自循环重定向：最多 5 跳（第 6 个请求触发上限），不得无限跟随。
    let error = downloader
        .download(&server.url("/r-loop"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_too_many_redirects", "{error:?}");
    assert_eq!(server.count("GET", "/r-loop"), 6, "上限 5 跳");

    assert_eq!(second.count("GET", "/x.glb"), 0, "任何被拒目标都不得被请求");
    assert!(qa_blob_count(data_dir) == 0, "以上路径都不得落盘");
    assert!(qa_tmp_parts(data_dir).is_empty());
}

// ---------------------------------------------------------------------------
// B3. DNS 重绑定 / 混合答案 / 明文 / 地址类别
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_dns_rebinding_mixed_answers_and_address_classes_are_refused() {
    let server = QaServer::start(vec![qa_prefix_rule(
        "GET",
        "/m.glb",
        vec![qa_respond(200, &[], b"NEVER-SERVED")],
    )]);
    let dir = qa_download_dir("qa13-rebind");
    let data_dir = dir.path();

    // 允许域解析到私网 → 拒绝且 0 连接。
    let downloader =
        ModelDownloader::new(data_dir, qa_policy(&["rebind.qa13.test"], 1_048_576)).with_resolver(
            Arc::new(QaResolver::new(&[("rebind.qa13.test", &["10.1.2.3"])])),
        );
    let error = downloader
        .download(&format!("http://rebind.qa13.test:{}/m.glb", server.port()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert_eq!(server.connections(), 0, "重绑定目标不得被连接");

    // 混合答案（公网 + 私网）整体拒绝（不能只看第一条）。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["rebind.qa13.test"], 1_048_576))
        .with_resolver(Arc::new(QaResolver::new(&[(
            "rebind.qa13.test",
            &["93.184.216.34", "192.168.1.9"],
        )])));
    let error = downloader
        .download(&format!("http://rebind.qa13.test:{}/m.glb", server.port()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert_eq!(server.connections(), 0);

    // 链路本地 / CGNAT / 组播 / 文档保留 / 基准测试 / 保留网段。
    for (host_ip, keyword) in [
        ("169.254.9.9", "链路本地"),
        ("100.64.0.5", "运营商级 NAT"),
        ("224.0.0.7", "组播"),
        ("192.0.2.44", "文档保留"),
        ("198.19.1.1", "基准测试"),
        ("245.1.1.1", "保留地址"),
    ] {
        let downloader = ModelDownloader::new(data_dir, qa_policy(&[host_ip], 1_048_576));
        let error = downloader
            .download(&format!("http://{host_ip}/m.glb"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "download_forbidden_address", "{host_ip}");
        assert!(
            error.message().contains(keyword),
            "{host_ip} 的拒绝理由应含「{keyword}」：{error:?}"
        );
    }

    // 回环：测试构建 + 显式放行时允许（两道门），未配置放行时拒绝。
    let mut open = qa_policy(&["127.0.0.1"], 1_048_576);
    open.allow_local_fixture = true;
    assert!(
        ModelDownloader::new(data_dir, open)
            .policy()
            .local_fixture_allowed(),
        "测试构建 + 显式配置应放行回环"
    );
    let mut closed = qa_policy(&["127.0.0.1"], 1_048_576);
    closed.allow_local_fixture = false;
    let downloader = ModelDownloader::new(data_dir, closed);
    assert!(!downloader.policy().local_fixture_allowed());
    let error = downloader
        .download("https://127.0.0.1:9/m.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert!(error.message().contains("回环地址"), "{error:?}");

    // 明文 http 在生产策略下拒绝（scheme 检查先于允许域）。
    let mut plain = qa_policy(&["plain.qa13.test"], 1_048_576);
    plain.allow_local_fixture = false;
    let downloader = ModelDownloader::new(data_dir, plain).with_resolver(Arc::new(
        QaResolver::new(&[("plain.qa13.test", &["127.0.0.1"])]),
    ));
    let error = downloader
        .download(&format!("http://plain.qa13.test:{}/m.glb", server.port()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_insecure_scheme");
    assert_eq!(server.connections(), 0);

    // 空名单 = 拒绝一切（不猜测 CDN 域名）。
    let downloader = ModelDownloader::new(data_dir, DownloadPolicy::default());
    let error = downloader
        .download("https://cdn.any.invalid/m.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_host_not_allowed");

    // URL 含账号密码 → 非法。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1"], 1_048_576));
    let error = downloader
        .download(&format!(
            "http://user:pass@127.0.0.1:{}/m.glb",
            server.port()
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_invalid_url");

    // 解析失败（解析器无该主机）→ 明确错误，不发起连接。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["missing.qa13.test"], 1_048_576))
        .with_resolver(Arc::new(QaResolver::new(&[])));
    let error = downloader
        .download("http://missing.qa13.test/m.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_dns");

    assert_eq!(
        server.count("GET", "/m.glb"),
        0,
        "全部负例都不得触达 fixture"
    );
    assert!(qa_blob_count(data_dir) == 0);
    assert!(qa_tmp_parts(data_dir).is_empty());
}

// ---------------------------------------------------------------------------
// B4. 大小两道门 / 截断重试（整文件重下）/ 磁盘满
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_size_gates_truncation_retry_and_disk_full() {
    let payload = qa_valid_bytes();
    let expected_sha = sha256_hex(&payload);
    let half = payload.len() / 2;
    let server = QaServer::start(vec![
        // 声明长度远大于上限（body 很小）。
        qa_rule(
            "GET",
            "/declared-large.glb",
            vec![qa_respond(
                200,
                &[("content-length", "8388608")],
                &payload[..16],
            )],
        ),
        // 无 Content-Length（chunked）但流总字节超上限。
        qa_rule(
            "GET",
            "/chunked-large.glb",
            vec![QaCanned::Chunked {
                status: 200,
                body: vec![0x5A; 8192],
                chunk: 1024,
            }],
        ),
        // 截断一次（半关闭），之后给完整响应（重试）。
        qa_rule(
            "GET",
            "/flaky.glb",
            vec![
                QaCanned::HalfClose {
                    status: 200,
                    body: payload.clone(),
                    truncate: half,
                },
                qa_respond(200, &[], &payload),
            ],
        ),
        // 正常文件（磁盘满用）。
        qa_rule("GET", "/ok.glb", vec![qa_respond(200, &[], &payload)]),
    ]);

    let dir = qa_download_dir("qa13-size-retry");
    let data_dir = dir.path();

    // 1) 声明长度 > 上限 → download_too_large（declared），不落盘。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1"], 1024));
    let error = downloader
        .download(&server.url("/declared-large.glb"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_too_large");
    assert!(matches!(
        error,
        DownloadError::TooLarge {
            declared: Some(_),
            ..
        }
    ));
    assert_eq!(qa_blob_count(data_dir), 0);
    assert!(qa_tmp_parts(data_dir).is_empty());

    // 2) chunked 流超上限 → download_too_large（流中计数这一道），不落盘。
    let error = downloader
        .download(&server.url("/chunked-large.glb"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_too_large");
    assert!(matches!(
        error,
        DownloadError::TooLarge { declared: None, .. }
    ));
    assert_eq!(qa_blob_count(data_dir), 0);
    assert!(qa_tmp_parts(data_dir).is_empty());

    // 3) 截断 → download_transport（可安全重试）；重试整文件重下（无 Range），最终哈希正确。
    let error = downloader
        .download(&server.url("/flaky.glb"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert!(error.is_retryable(), "传输中断必须可安全重试");
    assert_eq!(qa_blob_count(data_dir), 0, "中断不得留下 blob");
    assert!(qa_tmp_parts(data_dir).is_empty());

    let downloaded = downloader
        .download(&server.url("/flaky.glb"))
        .await
        .expect("重试成功");
    assert_eq!(downloaded.sha256, expected_sha);
    assert_eq!(downloaded.size, payload.len() as u64);
    assert_eq!(std::fs::read(&downloaded.path).unwrap(), payload);
    let flaky_requests = server
        .requests()
        .into_iter()
        .filter(|request| request.path == "/flaky.glb")
        .collect::<Vec<_>>();
    assert_eq!(flaky_requests.len(), 2);
    assert!(
        flaky_requests[1].header("range").is_none(),
        "重试为整文件重下（不得使用 Range 续传）"
    );
    assert_eq!(qa_blob_count(data_dir), 1);
    assert!(qa_tmp_parts(data_dir).is_empty());

    // 4) 磁盘满（注入空间探测）→ download_insufficient_storage，不落盘。
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1"], 1_048_576))
        .with_space_probe(SpaceProbe::Fixed(0));
    let error = downloader
        .download(&server.url("/ok.glb"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_insufficient_storage");
    assert!(error.message().contains("未保存任何资产"), "{error:?}");
    assert_eq!(qa_blob_count(data_dir), 1, "磁盘满不得新增 blob");
    assert!(qa_tmp_parts(data_dir).is_empty());

    // 5) 断连（服务器不发任何字节）→ 可重试分类。
    let silent = QaServer::start(vec![qa_rule(
        "GET",
        "/silent.glb",
        vec![QaCanned::Disconnect],
    )]);
    let downloader = ModelDownloader::new(data_dir, qa_policy(&["127.0.0.1"], 1_048_576));
    let error = downloader
        .download(&silent.url("/silent.glb"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert!(error.is_retryable());
    assert!(qa_tmp_parts(data_dir).is_empty());
}

// ---------------------------------------------------------------------------
// C. 执行器端到端（QA 自建流程与 fixture；负例在**刷新链接**与**重绑定**路径上）
// ---------------------------------------------------------------------------

const QA_PASSWORD: &str = "qa13-independent-password-77c1";
const QA_PRESET: &str = "tripo-h-v3.1-standard";
const QA_TASK_ID: &str = "qa13-task-1";

fn qa_fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("读取样例 {name} 失败：{error}"))
}

fn qa_price_catalog() -> (String, everything_manual::generation::catalog::PriceCatalog) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../price-catalog.example.toml");
    let text = std::fs::read_to_string(&path).expect("读取价格目录示例");
    let parsed = everything_manual::generation::catalog::parse(&text).expect("价格目录可解析");
    (path.to_string_lossy().into_owned(), parsed)
}

fn qa_multipart(
    purpose: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!("----qa13-{purpose}-{filename}");
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
             Content-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// 已配置 Provider（假凭据）+ 允许本机 fixture 的应用。
async fn qa_app(tag: &str, api_base: &str) -> (common::TestApp, String, String) {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    let mut tripo = common::configured_tripo("qa13-fake-tripo-key");
    tripo.base_url = api_base.to_owned();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = everything_manual::config::ProviderSettings {
        name: "manual_ai",
        base_url: "https://api.openai.invalid/v1".to_owned(),
        model: Some("gpt-5-mini".to_owned()),
        api_key: Some(SecretString::new("qa13-fake-manual-ai-key")),
        key_source: Some("QA 测试注入".to_owned()),
    };
    settings.download.allowed_hosts = vec!["127.0.0.1".to_owned()];
    settings.download.allow_local_fixture = true;
    let (catalog_path, catalog) = qa_price_catalog();
    settings.price_catalog_path = Some(std::path::PathBuf::from(catalog_path));
    settings.price_catalog = Some(catalog);
    let app = common::TestApp::with_settings(dir, settings).await;
    app.set_admin_password(QA_PASSWORD).await;
    let login = app
        .call(axum::http::Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": QA_PASSWORD }))
        .send()
        .await;
    assert_eq!(login.status, axum::http::StatusCode::OK, "{}", login.text());
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .expect("csrfToken")
        .to_owned();
    let cookie = login.session_cookie();
    (app, cookie, csrf)
}

/// 走产品 API 建出一个 ready 的可执行任务（QA 自写流程）。
async fn qa_create_job(app: &common::TestApp, cookie: &str, csrf: &str) -> (String, String) {
    use axum::http::{Method, StatusCode};

    let item = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "QA13 独立验收物品", "model": "QA13-X" }))
        .send()
        .await;
    assert_eq!(item.status, StatusCode::CREATED, "{}", item.text());
    let item_id = item.json()["data"]["id"].as_str().unwrap().to_owned();

    let upload = |purpose: &str, filename: &str, content_type: &str, bytes: &[u8]| {
        let (content_type_header, body) = qa_multipart(purpose, filename, content_type, bytes);
        let item_id = item_id.clone();
        let cookie = cookie.to_owned();
        let csrf = csrf.to_owned();
        async move {
            let response = app
                .call(Method::POST, &format!("/api/v1/items/{item_id}/assets"))
                .cookie(&cookie)
                .csrf(&csrf)
                .raw_body(Some(&content_type_header), body)
                .send()
                .await;
            assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
            response.json()["data"]["id"].as_str().unwrap().to_owned()
        }
    };

    let pdf = qa_fixture_bytes("sample-manual-text.pdf");
    let document_asset = upload("document", "manual.pdf", "application/pdf", &pdf).await;
    let document = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceAssetId": document_asset, "title": "QA13 说明书" }))
        .send()
        .await;
    assert_eq!(document.status, StatusCode::CREATED, "{}", document.text());
    let document_id = document.json()["data"]["id"].as_str().unwrap().to_owned();
    let source_sha = document.json()["data"]["sourceSha256"]
        .as_str()
        .unwrap()
        .to_owned();

    let preparation = app
        .call(
            Method::POST,
            &format!("/api/v1/documents/{document_id}/preparations"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceSha256": source_sha }))
        .send()
        .await;
    assert_eq!(
        preparation.status,
        StatusCode::CREATED,
        "{}",
        preparation.text()
    );
    let preparation_id = preparation.json()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let page_jpeg = qa_fixture_bytes("sample-photo-front.jpg");
    for page in 1..=2 {
        let text = upload(
            "pageText",
            "page.txt",
            "text/plain",
            format!("qa13-page-text-{page}").repeat(8).as_bytes(),
        )
        .await;
        let image = upload("pageImage", "page.jpg", "image/jpeg", &page_jpeg).await;
        let response = app
            .call(
                Method::PUT,
                &format!("/api/v1/preparations/{preparation_id}/pages/{page}"),
            )
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({
                "textAssetId": text,
                "imageAssetId": image,
                "viewport": { "width": 1240, "height": 1754, "rotation": 0 },
            }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    }
    let current = app
        .call(
            Method::GET,
            &format!("/api/v1/preparations/{preparation_id}"),
        )
        .cookie(cookie)
        .send()
        .await;
    let etag = current.header("etag").expect("准备 ETag");
    let completed = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation_id}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", &etag)
        .json(&json!({ "pageCount": 2 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    let mut photo_ids = Vec::new();
    for (view, name, content_type) in [
        ("front", "sample-photo-front.jpg", "image/jpeg"),
        ("left", "sample-photo-left.png", "image/png"),
    ] {
        let bytes = qa_fixture_bytes(name);
        let asset = upload("photo", name, content_type, &bytes).await;
        let response = app
            .call(Method::POST, &format!("/api/v1/items/{item_id}/photos"))
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "assetId": asset, "view": view }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        photo_ids.push(response.json()["data"]["id"].as_str().unwrap().to_owned());
    }

    let estimate = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/estimates"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({
            "preparationId": preparation_id,
            "photoIds": photo_ids,
            "modelPreset": QA_PRESET,
        }))
        .send()
        .await;
    assert_eq!(estimate.status, StatusCode::CREATED, "{}", estimate.text());
    let quote_id = estimate.json()["data"]["id"].as_str().unwrap().to_owned();

    let confirmed = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item_id}/estimates/{quote_id}/confirm"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .send()
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());

    let job = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/jobs"))
        .cookie(cookie)
        .csrf(csrf)
        .header("idempotency-key", "qa13-independent-job-key-1")
        .json(&json!({
            "quoteId": quote_id,
            "preparationId": preparation_id,
            "photoIds": photo_ids,
            "limits": { "tripoCreditMinor": 30_000, "manualAiUsdMicros": 500_000 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    (
        job.json()["data"]["id"].as_str().unwrap().to_owned(),
        item_id,
    )
}

fn qa_executor(
    app: &common::TestApp,
    clock: Arc<ManualClock>,
    injected: Option<(ModelDownloader, GlbBudget)>,
) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let handlers = TripoHandlers::from_settings(&settings).expect("已配置 Tripo 应可构造");
    let handlers = match injected {
        Some((downloader, budget)) => handlers.with_download(downloader, budget),
        None => handlers,
    };
    let mut registry = StageRegistry::new();
    handlers.register(&mut registry);
    fixed_jitter_executor(
        app.state().database().pool().clone(),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

async fn qa_stage(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    stages_repo::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .unwrap_or_else(|| panic!("阶段不存在：{}", kind.as_str()))
}

async fn qa_tick_until_stage(
    pool: &SqlitePool,
    executor: &Arc<JobExecutor>,
    clock: &Arc<ManualClock>,
    job_id: &str,
    kind: StageKind,
    want: JobStatus,
    max_ticks: usize,
) -> JobStage {
    for _ in 0..max_ticks {
        let stage = qa_stage(pool, job_id, kind).await;
        if stage.status == want {
            return stage;
        }
        clock.advance_millis(20_000);
        let _ = executor.tick().await.expect("tick");
    }
    let stage = qa_stage(pool, job_id, kind).await;
    panic!(
        "阶段 {} 未在 {max_ticks} tick 内达到 {}（实际 {}；needs_input={:?}；last_error={:?}）",
        kind.as_str(),
        want.as_str(),
        stage.status.as_str(),
        stage.needs_input_json,
        stage.last_error
    );
}

/// 全库文本列扫描：断言 canary（临时签名 URL 的查询串）没有被持久化为永久地址。
/// 返回 `(表, 列, 命中行数)`。
async fn qa_canary_hits(pool: &SqlitePool, canary: &str) -> Vec<(String, String, i64)> {
    let mut conn = pool.acquire().await.expect("连接");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("表清单");
    let mut hits = Vec::new();
    for table in tables {
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
            .bind(&table)
            .fetch_all(&mut *conn)
            .await
            .expect("列清单");
        for column in columns {
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM {table} WHERE instr(CAST({column} AS TEXT), ?) > 0"
            )))
            .bind(canary)
            .fetch_one(&mut *conn)
            .await
            .expect("扫描 canary");
            if count > 0 {
                hits.push((table.clone(), column, count));
            }
        }
    }
    hits
}

fn qa_upload_steps() -> Vec<QaCanned> {
    vec![qa_json_respond(&json!({
        "code": 0,
        "data": { "file_token": "qa13-token-1" }
    }))]
}

fn qa_submit_rule() -> QaRule {
    qa_repeat_rule(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![qa_json_respond(&json!({
            "code": 0,
            "data": { "task_id": QA_TASK_ID }
        }))],
    )
}

fn qa_task_success(model_url: &str) -> Value {
    json!({
        "code": 0,
        "data": {
            "task_id": QA_TASK_ID,
            "status": "success",
            "progress": 100,
            "credits_consumed": 30,
            "output": { "model_url": model_url }
        }
    })
}

// ---------------------------------------------------------------------------
// C1. 链接过期 → 重新查询已知任务；**刷新取回的新链接仍必须重新做 SSRF 校验**
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_refreshed_link_is_revalidated_and_never_repurchases() {
    // 模型 CDN：旧链接 403（过期）。
    let cdn = QaServer::start(vec![qa_repeat_rule(
        "GET",
        "/m-old.glb",
        vec![qa_respond(403, &[], b"expired")],
    )]);
    let stale_url = cdn.url("/m-old.glb?sign=qa13-canary-stale");
    // 刷新后的链接指向**不在允许域名单**内的主机（供应商/攻击者可控的形态）。
    let refreshed_url = format!(
        "http://not-allowed.qa13.test:{}/m-new.glb?sign=qa13-canary-fresh",
        cdn.port()
    );
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_prefix_rule(
            "GET",
            "/v3/tasks/",
            vec![
                qa_json_respond(&qa_task_success(&stale_url)),
                qa_json_respond(&qa_task_success(&refreshed_url)),
            ],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa13-c1", &api_base).await;
    let (job_id, item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock), None);
    let pool = app.state().database().pool().clone();

    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::NeedsInput,
        120,
    )
    .await;

    // 阶段结论：刷新后的链接被允许域校验拒绝（不是下载成功）。
    let needs_input = stage
        .needs_input_json
        .clone()
        .unwrap_or_default()
        .to_string();
    assert!(
        needs_input.contains("download_host_not_allowed"),
        "刷新后的链接必须重新做允许域校验：{needs_input}"
    );
    // 旧的过期链接请求 1 次；刷新后的主机从未被连接。
    assert_eq!(cdn.count("GET", "/m-old.glb"), 1);
    assert_eq!(cdn.connections(), 1, "只允许连接过期链接一次");
    assert_eq!(cdn.count("GET", "/m-new.glb"), 0);
    // 重新查询已知任务（轮询 1 次 + 过期后重查 1 次），付费提交恒为 1 次。
    assert!(
        api.requests()
            .iter()
            .filter(|request| request.path.starts_with("/v3/tasks/"))
            .count()
            >= 2,
        "必须重新查询已知任务取新链接"
    );
    assert_eq!(
        api.count("POST", "/v3/generation/multiview-to-model"),
        1,
        "不得重新购买"
    );
    // API 侧只允许三类请求：上传（POST /v3/files）、付费提交（恰 1 次）、任务查询（GET）。
    for request in api.requests() {
        let allowed = (request.method == "POST" && request.path == "/v3/files")
            || (request.method == "POST" && request.path == "/v3/generation/multiview-to-model")
            || (request.method == "GET" && request.path.starts_with("/v3/tasks/"));
        assert!(
            allowed,
            "出现非预期 API 请求：{} {}",
            request.method, request.target
        );
    }
    for request in cdn.requests() {
        assert!(
            request.header("authorization").is_none(),
            "CDN 请求不得带 Authorization"
        );
    }
    // 没有任何模型资产/版本（下载未成功）；临时链接只允许出现在 tripo_poll 观察事实里。
    let hits = qa_canary_hits(&pool, "qa13-canary-stale").await;
    let unexpected: Vec<_> = hits
        .iter()
        .filter(|(table, column, _)| !(table == "job_stages" && column == "usage_json"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "非预期位置出现签名：{unexpected:?}（全部：{hits:?}）"
    );
    let mut counter = pool.acquire().await.unwrap();
    let model_assets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE purpose = 'model'")
            .fetch_one(&mut *counter)
            .await
            .unwrap();
    assert_eq!(model_assets, 0);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_revisions")
        .fetch_one(&mut *counter)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    assert!(qa_tmp_parts(app.dir()).is_empty());
    let _ = item_id;
}

// ---------------------------------------------------------------------------
// C2. DNS 重绑定（执行器路径）：允许域解析到私网 → needs_input、0 次 CDN 连接
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_rebinding_at_executor_level_blocks_download() {
    let cdn = QaServer::start(vec![qa_repeat_rule(
        "GET",
        "/m.glb",
        vec![qa_respond(200, &[], b"NEVER")],
    )]);
    let rebinding_url = format!(
        "http://cdn.qa13.test:{}/m.glb?sign=qa13-canary-rebind",
        cdn.port()
    );
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_prefix_rule(
            "GET",
            "/v3/tasks/",
            vec![qa_json_respond(&qa_task_success(&rebinding_url))],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa13-c2", &api_base).await;
    let (job_id, _item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let settings = app.state().settings().clone();
    let mut policy = DownloadPolicy::from_settings(&settings);
    policy.allowed_hosts = vec!["cdn.qa13.test".to_owned()];
    policy.allow_local_fixture = true;
    let downloader = ModelDownloader::new(app.dir(), policy).with_resolver(Arc::new(
        QaResolver::new(&[("cdn.qa13.test", &["10.1.2.3"])]),
    ));
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(
        &app,
        Arc::clone(&clock),
        Some((downloader, GlbBudget::default())),
    );
    let pool = app.state().database().pool().clone();

    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::NeedsInput,
        120,
    )
    .await;
    let needs_input = stage
        .needs_input_json
        .clone()
        .unwrap_or_default()
        .to_string();
    assert!(
        needs_input.contains("download_forbidden_address"),
        "重绑定必须被地址校验拒绝：{needs_input}"
    );
    assert_eq!(cdn.connections(), 0, "私网目标不得被连接");
    assert_eq!(cdn.count("GET", "/m.glb"), 0);
    assert_eq!(
        api.count("POST", "/v3/generation/multiview-to-model"),
        1,
        "地址被拒也不得重新购买"
    );
    let mut counter = pool.acquire().await.unwrap();
    let model_assets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE purpose = 'model'")
            .fetch_one(&mut *counter)
            .await
            .unwrap();
    assert_eq!(model_assets, 0);
    assert!(qa_tmp_parts(app.dir()).is_empty());
}

// ---------------------------------------------------------------------------
// C3. 超预算：保留原始模型字节 + rejected revision + 不自动降预算/不重下/不重购
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_over_budget_preserves_original_bytes_and_never_repurchases() {
    let payload = qa_valid_bytes();
    let expected_sha = sha256_hex(&payload);
    let payload_for_server = payload.clone();
    let canary = "qa13-canary-budget-signature";
    let cdn = QaServer::start(vec![qa_repeat_rule(
        "GET",
        "/model.glb",
        vec![QaCanned::Respond {
            status: 200,
            headers: vec![("content-type".to_owned(), "model/gltf-binary".to_owned())],
            body: payload_for_server,
        }],
    )]);
    let model_url = cdn.url(&format!("/model.glb?sign={canary}"));
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_prefix_rule(
            "GET",
            "/v3/tasks/",
            vec![qa_json_respond(&qa_task_success(&model_url))],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa13-c3", &api_base).await;
    let (job_id, item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let settings = app.state().settings().clone();
    let downloader = ModelDownloader::new(app.dir(), DownloadPolicy::from_settings(&settings));
    // 注入更小预算（2 面模型 vs 上限 1 面）：覆盖超预算路径。
    let budget = GlbBudget {
        max_triangles: 1,
        max_texture_dimension: 4096,
    };
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock), Some((downloader, budget)));
    let pool = app.state().database().pool().clone();

    qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        120,
    )
    .await;
    let validate = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        60,
    )
    .await;

    // 1) 缺项代码与文案：可行动、保留原始模型、无"降低面数/自动修复"入口。
    let needs_input = validate
        .needs_input_json
        .clone()
        .unwrap_or_default()
        .to_string();
    assert!(needs_input.contains("gltf_face_limit"), "{needs_input}");
    assert!(needs_input.contains("原始模型已保留"), "{needs_input}");
    assert!(needs_input.contains("未自动降预算"), "{needs_input}");
    assert!(!needs_input.contains("自动修复"), "{needs_input}");

    // 2) 原始模型字节必须在本地保留且逐字节等于服务端提供的字节。
    let blob_path = app
        .dir()
        .join("blobs")
        .join(&expected_sha[..2])
        .join(&expected_sha);
    assert!(
        blob_path.is_file(),
        "原始模型必须保留：{}",
        blob_path.display()
    );
    assert_eq!(
        std::fs::read(&blob_path).unwrap(),
        payload,
        "原始模型不得被修改"
    );

    // 3) rejected revision（bounds 为空，指向同一 blob）。
    let mut conn = pool.acquire().await.unwrap();
    let (state, bounds, revision_sha, asset_id): (String, Option<String>, String, String) =
        sqlx::query_as(
            "SELECT validation_state, bounds, sha256, asset_id FROM model_revisions WHERE item_id = ?",
        )
        .bind(&item_id)
        .fetch_one(&mut *conn)
        .await
        .expect("rejected revision 必须存在");
    assert_eq!(state, "rejected");
    assert!(
        bounds.is_none(),
        "rejected revision 不得写 bounds：{bounds:?}"
    );
    assert_eq!(revision_sha, expected_sha);
    let (purpose, blob_id): (String, String) =
        sqlx::query_as("SELECT purpose, blob_id FROM assets WHERE id = ?")
            .bind(&asset_id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(purpose, "model");
    assert_eq!(blob_id, expected_sha);

    // 4) 不自动重下、不重新购买：CDN 只被请求 1 次，付费提交 1 次。
    assert_eq!(cdn.count("GET", "/model.glb"), 1);
    assert_eq!(api.count("POST", "/v3/generation/multiview-to-model"), 1);

    // 5) 临时签名 URL 不作为永久地址保存：全库文本列扫描 canary。
    //    T13 自己的位置（assets / model_revisions / 下载与校验阶段 usage）必须 0 命中；
    //    唯一允许的例外是 `tripo_poll` 的观察事实（T12 设计：modelUrl 供下载阶段取用，
    //    出网脱敏属 T15/T17；见 ADR-022 第 8 条与 QA 回合 14 记录）。
    let hits = qa_canary_hits(&pool, canary).await;
    let unexpected: Vec<_> = hits
        .iter()
        .filter(|(table, column, _)| !(table == "job_stages" && column == "usage_json"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "签名串出现在 T13 不应保存的位置：{unexpected:?}（全部命中：{hits:?}）"
    );
    let usage_hit_stages: Vec<String> = sqlx::query_scalar(
        "SELECT stage_kind FROM job_stages WHERE job_id = ? AND instr(CAST(usage_json AS TEXT), ?) > 0 \
         ORDER BY stage_kind",
    )
    .bind(&job_id)
    .bind(canary)
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert!(
        usage_hit_stages.iter().all(|kind| kind == "tripo_poll"),
        "只有 tripo_poll 的观察事实可以保留临时链接：{usage_hit_stages:?}"
    );
    println!("QA13 canary_hits={hits:?} usage_hit_stages={usage_hit_stages:?}");
    // 阶段 usage 允许记录 host 摘要，但不得含完整 URL。
    for stage_kind in ["model_download", "model_validate"] {
        let usage: Option<String> = sqlx::query_scalar(
            "SELECT usage_json FROM job_stages WHERE job_id = ? AND stage_kind = ?",
        )
        .bind(&job_id)
        .bind(stage_kind)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        let usage = usage.unwrap_or_default();
        assert!(
            !usage.contains("http"),
            "{stage_kind} usage 不得含 URL：{usage}"
        );
        assert!(
            !usage.contains(canary),
            "{stage_kind} usage 不得含签名：{usage}"
        );
    }

    // 6) 重跑校验阶段：不产生第二行 revision（幂等），不重新下载。
    let stage_row = qa_stage(&pool, &job_id, StageKind::ModelValidate).await;
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "UPDATE job_stages SET status = 'queued', lease_owner = NULL, lease_until = NULL, \
         next_run_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&stage_row.id)
    .execute(&mut *conn)
    .await
    .unwrap();
    qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        20,
    )
    .await;
    let revisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM model_revisions WHERE item_id = ?")
            .bind(&item_id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(revisions, 1, "同一内容重跑不得产生第二行 revision");
    assert_eq!(cdn.count("GET", "/model.glb"), 1, "重跑不得重新下载");
    assert_eq!(std::fs::read(&blob_path).unwrap(), payload);
    assert!(qa_tmp_parts(app.dir()).is_empty());
}

// ---------------------------------------------------------------------------
// C4. 正常链路：下载 → 校验 → **validated 不可变 revision**（bounds 来自 QA 自建字节）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_valid_model_creates_immutable_validated_revision() {
    let payload = qa_valid_bytes();
    let expected_sha = sha256_hex(&payload);
    let canary = "qa13-canary-valid-signature";
    let cdn = QaServer::start(vec![qa_repeat_rule(
        "GET",
        "/model.glb",
        vec![QaCanned::Respond {
            status: 200,
            headers: vec![("content-type".to_owned(), "model/gltf-binary".to_owned())],
            body: payload.clone(),
        }],
    )]);
    let model_url = cdn.url(&format!("/model.glb?sign={canary}"));
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_prefix_rule(
            "GET",
            "/v3/tasks/",
            vec![qa_json_respond(&qa_task_success(&model_url))],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa13-c4", &api_base).await;
    let (job_id, item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let settings = app.state().settings().clone();
    let downloader = ModelDownloader::new(app.dir(), DownloadPolicy::from_settings(&settings));
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(
        &app,
        Arc::clone(&clock),
        Some((downloader, GlbBudget::default())),
    );
    let pool = app.state().database().pool().clone();

    let download = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        120,
    )
    .await;
    let validate = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        60,
    )
    .await;

    // 下载阶段：sha256/大小与 QA 字节一致、无 URL、非复用。
    let download_usage = download.usage_json.clone().unwrap_or_default();
    assert_eq!(download_usage["sha256"], expected_sha);
    assert_eq!(download_usage["sizeBytes"], payload.len() as u64);
    assert_eq!(download_usage["linkRefreshed"], false);
    assert!(
        !download_usage.to_string().contains("http"),
        "{download_usage}"
    );

    // 校验阶段：validated + bounds 与 QA 字节的 AABB 一致。
    let validate_usage = validate.usage_json.clone().unwrap_or_default();
    assert_eq!(validate_usage["validation"], "validated");
    assert_eq!(validate_usage["triangles"], 2);
    assert_eq!(validate_usage["vertices"], 4);
    assert_eq!(validate_usage["maxTextureDimension"], 8);
    assert_eq!(validate_usage["bounds"]["min"], json!([-1.0, -1.0, 0.0]));
    assert_eq!(validate_usage["bounds"]["max"], json!([1.0, 1.0, 0.0]));

    // 不可变 revision：一行、validated、有 bounds、关联付费 attempt、指向同一 blob。
    let mut conn = pool.acquire().await.unwrap();
    let (revision_id, state, bounds, sha, asset_id, attempt): (
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT id, validation_state, bounds, sha256, asset_id, provider_attempt_id \
         FROM model_revisions WHERE item_id = ?",
    )
    .bind(&item_id)
    .fetch_one(&mut *conn)
    .await
    .expect("validated revision 必须存在");
    assert_eq!(state, "validated");
    assert_eq!(sha, expected_sha);
    assert!(
        attempt.is_some(),
        "validated revision 应关联产生它的付费 attempt"
    );
    let bounds: Value = serde_json::from_str(&bounds.expect("validated 必须有 bounds")).unwrap();
    assert_eq!(bounds["min"], json!([-1.0, -1.0, 0.0]));
    assert_eq!(bounds["max"], json!([1.0, 1.0, 0.0]));
    assert_eq!(bounds["triangles"], 2);

    // 模型字节不可变：blob 文件 = QA 提供的字节，且 asset 指向它。
    let blob_path = app
        .dir()
        .join("blobs")
        .join(&expected_sha[..2])
        .join(&expected_sha);
    assert_eq!(std::fs::read(&blob_path).unwrap(), payload);
    let (purpose, blob_id): (String, String) =
        sqlx::query_as("SELECT purpose, blob_id FROM assets WHERE id = ?")
            .bind(&asset_id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(purpose, "model");
    assert_eq!(blob_id, expected_sha);

    // 临时 URL 只允许出现在 tripo_poll 观察事实。
    let hits = qa_canary_hits(&pool, canary).await;
    let unexpected: Vec<_> = hits
        .iter()
        .filter(|(table, column, _)| !(table == "job_stages" && column == "usage_json"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "非预期位置出现签名：{unexpected:?}（全部：{hits:?}）"
    );

    // 重跑校验：不产生第二行、不覆盖既有 bounds（幂等且不可变）。
    let stage_row = qa_stage(&pool, &job_id, StageKind::ModelValidate).await;
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "UPDATE job_stages SET status = 'queued', lease_owner = NULL, lease_until = NULL, \
         next_run_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&stage_row.id)
    .execute(&mut *conn)
    .await
    .unwrap();
    qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        20,
    )
    .await;
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, bounds FROM model_revisions WHERE item_id = ?")
            .bind(&item_id)
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1, "重跑不得产生第二行 revision");
    assert_eq!(rows[0].0, revision_id);
    assert_eq!(rows[0].1.as_deref(), Some(bounds.to_string().as_str()));
    assert_eq!(cdn.count("GET", "/model.glb"), 1, "重跑不得重新下载");
    assert!(qa_tmp_parts(app.dir()).is_empty());
}

// ---------------------------------------------------------------------------
// C5. 截断 GLB（下载成功、内容损坏）：needs_input + 保留原始字节 + 不自动重下
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_truncated_glb_at_handler_level_keeps_original() {
    let full = qa_valid_bytes();
    // 去掉文件尾 8 字节：头内声明长度仍指向原始大小 → glb_declared_length。
    let truncated = full[..full.len() - 8].to_vec();
    let truncated_sha = sha256_hex(&truncated);
    let served = truncated.clone();
    let cdn = QaServer::start(vec![qa_repeat_rule(
        "GET",
        "/model.glb",
        vec![QaCanned::Respond {
            status: 200,
            headers: vec![("content-type".to_owned(), "model/gltf-binary".to_owned())],
            body: served,
        }],
    )]);
    let model_url = cdn.url("/model.glb?sign=qa13-canary-truncated");
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_prefix_rule(
            "GET",
            "/v3/tasks/",
            vec![qa_json_respond(&qa_task_success(&model_url))],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa13-c5", &api_base).await;
    let (job_id, item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let settings = app.state().settings().clone();
    let downloader = ModelDownloader::new(app.dir(), DownloadPolicy::from_settings(&settings));
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(
        &app,
        Arc::clone(&clock),
        Some((downloader, GlbBudget::default())),
    );
    let pool = app.state().database().pool().clone();

    qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        120,
    )
    .await;
    let validate = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        60,
    )
    .await;

    let needs_input = validate
        .needs_input_json
        .clone()
        .unwrap_or_default()
        .to_string();
    assert!(
        needs_input.contains("glb_declared_length") || needs_input.contains("glb_chunk_layout"),
        "截断 GLB 必须以容器错误进入 needs_input：{needs_input}"
    );
    assert!(needs_input.contains("原始模型已保留"), "{needs_input}");
    assert!(needs_input.contains("未静默修改"), "{needs_input}");

    // 原始（截断）字节必须原样保留，不得被"修复"或重写。
    let blob_path = app
        .dir()
        .join("blobs")
        .join(&truncated_sha[..2])
        .join(&truncated_sha);
    assert!(
        blob_path.is_file(),
        "原始模型必须保留：{}",
        blob_path.display()
    );
    assert_eq!(std::fs::read(&blob_path).unwrap(), truncated);

    let mut conn = pool.acquire().await.unwrap();
    let (state, bounds): (String, Option<String>) =
        sqlx::query_as("SELECT validation_state, bounds FROM model_revisions WHERE item_id = ?")
            .bind(&item_id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(state, "rejected");
    assert!(bounds.is_none());

    // 不重新下载、不重新购买。
    assert_eq!(cdn.count("GET", "/model.glb"), 1);
    assert_eq!(api.count("POST", "/v3/generation/multiview-to-model"), 1);
    assert!(qa_tmp_parts(app.dir()).is_empty());
}

// ---------------------------------------------------------------------------
// A6. 观察（P3）：SOF 落在贴图首 64 KiB 之外的合法 JPEG 被判"尺寸不可判定"
// ---------------------------------------------------------------------------

#[test]
fn qa_jpeg_with_sof_beyond_the_64kib_window_is_rejected() {
    // 构造一个带最大 APP1 段 + APP2 段、SOF0 在 ~66.5 KiB 处的 JPEG。
    // 校验器只读 bufferView 的首 64 KiB（HEADER_WINDOW），因此判定不了尺寸。
    let mut jpeg = vec![0xFF, 0xD8];
    jpeg.extend_from_slice(&[0xFF, 0xE1]);
    let app1_len: u16 = 65_535;
    jpeg.extend_from_slice(&app1_len.to_be_bytes());
    jpeg.extend(std::iter::repeat_n(0_u8, app1_len as usize - 2));
    jpeg.extend_from_slice(&[0xFF, 0xE2]);
    let app2_len: u16 = 1_000;
    jpeg.extend_from_slice(&app2_len.to_be_bytes());
    jpeg.extend(std::iter::repeat_n(0_u8, app2_len as usize - 2));
    let sof_offset = jpeg.len();
    jpeg.extend_from_slice(&[0xFF, 0xC0]);
    jpeg.extend_from_slice(&17_u16.to_be_bytes());
    jpeg.push(8);
    jpeg.extend_from_slice(&12_u16.to_be_bytes());
    jpeg.extend_from_slice(&34_u16.to_be_bytes());
    jpeg.push(3);
    jpeg.extend(std::iter::repeat_n(0_u8, 15));
    assert!(
        sof_offset > 64 * 1024,
        "SOF 必须落在 64 KiB 窗口之外（实际偏移 {sof_offset}）"
    );

    let mut bin = qa_valid_bin();
    bin.truncate(QA_PNG_OFFSET);
    bin.extend_from_slice(&jpeg);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let json = qa_json(bin.len(), jpeg.len());
    let error = qa_inspect(&qa_assemble(&json, Some(&bin)), &GlbBudget::default()).unwrap_err();
    // 当前行为：fail-closed 拒绝（needs_input + 保留原件），但这是**合法 JPEG 被误拒**的形态。
    assert_eq!(error.code(), "gltf_image_invalid");
    assert!(error.message().contains("尺寸不可判定"), "{error:?}");
    println!("QA13 P3: 合法 JPEG 的 SOF 偏移 {sof_offset} > 64 KiB 读窗口 → 被判尺寸不可判定");
}

// ---------------------------------------------------------------------------
// C3. 回合 27 · BUG-009 复验：**失败路径**（传输错误 / HTTP 报错）的真实处理器
//     落库与任务详情 DTO 都不得出现完整签名 URL（AC-066、contracts §1）。
//
// 复用本文件（QA 回合 15 自建）的原始 TCP fixture 与建单流程（同一下载域），
// 断言与既有用例相互独立；修复前形态：`lastError` 含 `…?sign=<canary>`。
// 全程 127.0.0.1；零外网、零付费。
// ---------------------------------------------------------------------------

/// 判据（QA 回合 26/27 一致）：`://`、`sign=`、`/qa-r27/`、签名 canary 一律不得出现；
/// 允许保留：裸 host、sha256 摘要标签、task ID、状态码、结论文案。
fn qa27_assert_clean(text: &str, canary: &str, context: &str) {
    assert!(!text.contains("://"), "{context} 不得含 URL 形态：{text}");
    assert!(!text.contains(canary), "{context} 不得含签名：{text}");
    assert!(
        !text.contains("/qa-r27/"),
        "{context} 不得含供应商产物 path：{text}"
    );
}

/// BUG-009 回归（形态 1 · 传输错误）：`model_download` 的传输类失败（连接被拒 →
/// 生产：CDN 端口不可达/TCP RST）经**真实处理器**落库后，`job_stages.last_error`
/// 与任务详情 DTO 都不得含完整签名 URL；重试用尽转 `failed` 的文本同样干净。
#[tokio::test]
async fn qa_bug009_transport_failure_never_persists_signed_url() {
    use axum::http::{Method, StatusCode};

    const CANARY: &str = "qa27-canary-transport-signature-4b61";
    // 占位端口后立即释放 → 连接被拒（与 QA 回合 26 复现用例同一手法）。
    let listener = TcpListener::bind("127.0.0.1:0").expect("占位端口");
    let port = listener.local_addr().expect("本地地址").port();
    drop(listener);
    let dead_url =
        format!("http://127.0.0.1:{port}/qa-r27/model.glb?sign={CANARY}&expires=9999999999");

    let mut task_rule = qa_prefix_rule(
        "GET",
        "/v3/tasks/",
        vec![qa_json_respond(&qa_task_success(&dead_url))],
    );
    task_rule.repeat_last = true;
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        task_rule,
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa27-bug009-transport", &api_base).await;
    let (job_id, _item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock), None);
    let pool = app.state().database().pool().clone();

    // 第一次传输失败 → retry_wait（可安全重试；不产生供应商费用）。
    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::RetryWait,
        60,
    )
    .await;
    let last_error = stage.last_error.clone().unwrap_or_default();
    println!("QA27 transport last_error = {last_error}");
    assert!(
        last_error.contains("下载可安全重试"),
        "传输失败的结论必须保留：{last_error}"
    );
    assert!(
        last_error.contains("连接失败") || last_error.contains("传输失败"),
        "错误类别必须保留：{last_error}"
    );
    qa27_assert_clean(&last_error, CANARY, "retry_wait 的 last_error");

    // 安全重试用尽 → failed（同一文本的第二形态也要干净）。
    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Failed,
        60,
    )
    .await;
    let exhausted = stage.last_error.clone().unwrap_or_default();
    println!("QA27 exhausted last_error = {exhausted}");
    assert!(exhausted.contains("安全重试已用尽"), "{exhausted}");
    qa27_assert_clean(&exhausted, CANARY, "failed 的 last_error");

    // 全库扫描（逐表逐文本列）：签名不得落库到任何位置。
    let hits = qa_canary_hits(&pool, CANARY).await;
    assert!(hits.is_empty(), "全库出现签名：{hits:?}");

    // 任务详情 DTO：stage/attempt 的 lastError 与整体响应都必须干净。
    let detail = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let body = detail.json();
    let body_text = body.to_string();
    qa27_assert_clean(&body_text, CANARY, "任务详情响应体");
    let dto_error = body["data"]["stages"]
        .as_array()
        .expect("stages 数组")
        .iter()
        .find(|stage| stage["stageKind"] == "model_download")
        .and_then(|stage| stage["lastError"].as_str())
        .expect("model_download.lastError 非空（只替换 URL，不清空）")
        .to_owned();
    println!("QA27 dto lastError = {dto_error}");
    assert!(dto_error.contains("安全重试已用尽"), "{dto_error}");
    qa27_assert_clean(&dto_error, CANARY, "DTO lastError");

    // 恢复语义：不得重新购买；重试只走下载（免费）。
    assert_eq!(
        api.count("POST", "/v3/generation/multiview-to-model"),
        1,
        "不得重新购买"
    );
}

/// 与 [`qa_app`] 相同，但把 `manual_ai` 也指向本机 fixture（回合 27 覆盖提供方文本路径）。
async fn qa27_app_with_manual(
    tag: &str,
    api_base: &str,
    manual_base: &str,
) -> (common::TestApp, String, String) {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    let mut tripo = common::configured_tripo("qa27-fake-tripo-key");
    tripo.base_url = api_base.to_owned();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = everything_manual::config::ProviderSettings {
        name: "manual_ai",
        base_url: manual_base.to_owned(),
        model: Some("gpt-5-mini".to_owned()),
        api_key: Some(SecretString::new("qa27-fake-manual-ai-key")),
        key_source: Some("QA 测试注入".to_owned()),
    };
    settings.download.allowed_hosts = vec!["127.0.0.1".to_owned()];
    settings.download.allow_local_fixture = true;
    let (catalog_path, catalog) = qa_price_catalog();
    settings.price_catalog_path = Some(std::path::PathBuf::from(catalog_path));
    settings.price_catalog = Some(catalog);
    let app = common::TestApp::with_settings(dir, settings).await;
    app.set_admin_password(QA_PASSWORD).await;
    let login = app
        .call(axum::http::Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": QA_PASSWORD }))
        .send()
        .await;
    assert_eq!(login.status, axum::http::StatusCode::OK, "{}", login.text());
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .expect("csrfToken")
        .to_owned();
    let cookie = login.session_cookie();
    (app, cookie, csrf)
}

/// 生产接线（Tripo + 说明书 AI + 组装；与 `serve` 同源注册）。
fn qa27_pipeline_executor(app: &common::TestApp, clock: Arc<ManualClock>) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let mut registry = StageRegistry::new();
    everything_manual::providers::register_provider_handlers(&mut registry, &settings)
        .expect("两个 Provider 都已配置");
    let pipeline = everything_manual::jobs::PipelineHandlers::from_settings(&settings);
    pipeline.register(&mut registry);
    fixed_jitter_executor(
        app.state().database().pool().clone(),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

/// 回合 27 现场：说明书 AI 拒答（refusal 原文含 `?sign=<canary>`）→ `manual_extract`
/// 进入 `needs_input`。返回（app、pool、job_id、cookie、fixture 调用计数句柄）。
async fn qa27_run_manual_refusal(
    tag: &str,
    canary: &str,
) -> (common::TestApp, SqlitePool, String, String, Arc<QaServer>) {
    let refusal = json!({
        "id": "resp_qa27_refusal_0001",
        "object": "response",
        "created_at": 1757635200,
        "status": "completed",
        "model": "qa27-fixture",
        "output": [{
            "type": "message",
            "id": "msg_qa27_0001",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "refusal",
                "refusal": format!(
                    "无法从页图可靠识别；下载参考 https://cdn.qa27.invalid/p.png?sign={canary} 也失败"
                ),
            }],
        }],
        "usage": { "input_tokens": 100, "output_tokens": 10, "total_tokens": 110 },
    });
    let manual = Arc::new(QaServer::start(vec![qa_repeat_rule(
        "POST",
        "/v1/responses",
        vec![qa_json_respond(&refusal)],
    )]));
    let manual_base = format!("http://127.0.0.1:{}/v1", manual.port());

    // 模型分支：跑起来即可（本用例只关心知识分支的文本；下载目标不可达也不影响扫描）。
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        qa_repeat_rule(
            "GET",
            "/v3/tasks/",
            vec![qa_json_respond(&qa_task_success(
                "http://127.0.0.1:9/qa-r27/model.glb?sign=qa27-model-canary-0000",
            ))],
        ),
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa27_app_with_manual(tag, &api_base, &manual_base).await;
    let (job_id, _item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa27_pipeline_executor(&app, Arc::clone(&clock));
    let pool = app.state().database().pool().clone();

    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::NeedsInput,
        80,
    )
    .await;
    println!(
        "QA27 manual needs_input = {}",
        stage.needs_input_json.clone().unwrap_or_default()
    );
    qa27_assert_clean(
        &stage
            .needs_input_json
            .clone()
            .unwrap_or_default()
            .to_string(),
        canary,
        "needs_input_json",
    );
    (app, pool, job_id, cookie, manual)
}

/// **BUG-012（回合 27 新开，RED）**：说明书 AI 提供方文本（refusal）里的签名 URL 会被
/// **原样写入 `job_stages.usage_json` 的 `errorSummary`**（`handlers.rs` 的
/// `failure_of` → `BatchExtractionResult::not_produced` → 结果事实），而 `usage_json`
/// 在仓储写入路径**没有**走统一脱敏入口（`job_stages.rs` 的 `advance`/`set_result_fact`
/// 只脱敏 `last_error`/`needs_input_json`）。违反 ADR-032 第 1 条"URL 永不落库"。
/// 展示/备份侧仍被兜住（DTO `usage` 与备份 JSON 列会整串替换），因此暴露面 = data-dir 库文件。
///
/// **回合 28 复验转正**（QA）：RD 在产生侧（`failure_of`）与仓储侧（`advance`/`set_result_fact`）
/// 双层收口后本用例转绿，断言逐字未改（只去掉 `#[ignore]`；保留轨迹见 qa-report 回合 28）。
#[tokio::test]
async fn qa_bug012_provider_text_signed_url_must_not_be_persisted_into_usage_json() {
    const CANARY: &str = "qa27-manual-canary-5e42";
    let (_app, pool, _job_id, _cookie, manual) =
        qa27_run_manual_refusal("qa27-bug012-usage-json", CANARY).await;

    // 全库扫描：提供方文本里的签名不得落库到任何表/列。
    let hits = qa_canary_hits(&pool, CANARY).await;
    assert!(
        hits.is_empty(),
        "提供方文本里的签名落库：{hits:?}（ADR-032：临时 URL 永不落库）"
    );
    assert!(manual.count("POST", "/v1/responses") >= 1, "用例前提");
}

/// BUG-012 独立复验（QA 回合 28，全新 canary `qa28-…`）：新写入的 `usage_json`
/// 整列不得含 `://`/签名；同时核对可诊断性不回退——`errorSummary` 仍是字符串、
/// 句子尾部保留、`outcome`/`errorCode`/`responseId`/token 计数等事实字段保留。
#[tokio::test]
async fn qa_r28_usage_json_new_writes_never_contain_url_scheme() {
    use serde_json::Value;

    const CANARY: &str = "qa28-usage-canary-c0de";
    let (_app, pool, _job_id, _cookie, manual) =
        qa27_run_manual_refusal("qa28-bug012-usage-json", CANARY).await;

    // 全库扫描（回合 28 独立 canary）：签名不得出现在任何表/列。
    let hits = qa_canary_hits(&pool, CANARY).await;
    assert!(hits.is_empty(), "提供方文本里的签名落库：{hits:?}");

    let usage_text: String = sqlx::query_scalar(
        "SELECT usage_json FROM job_stages WHERE stage_kind = 'manual_extract' \
         ORDER BY batch_index LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("manual_extract.usage_json 必须已写入");
    assert!(
        !usage_text.contains("://"),
        "usage_json 不得含 URL 形态：{usage_text}"
    );
    assert!(
        !usage_text.contains(CANARY),
        "usage_json 不得含签名：{usage_text}"
    );

    let usage: Value = serde_json::from_str(&usage_text).expect("usage_json 仍是合法 JSON");
    let summary = usage["errorSummary"]
        .as_str()
        .expect("errorSummary 仍是字符串（不得整串变对象）");
    assert!(
        summary.ends_with("也失败"),
        "URL 之后的句子必须保留：{summary}"
    );
    assert!(
        summary.contains("临时供应商地址已脱敏"),
        "摘要标签保留（可诊断）：{summary}"
    );
    // 事实字段不得因脱敏丢失（计费/诊断/类别）。
    assert_eq!(usage["outcome"], "refusal", "结论保留：{usage}");
    assert_eq!(
        usage["errorCode"], "manual_ai_refusal",
        "错误类别保留：{usage}"
    );
    assert_eq!(
        usage["responseId"], "resp_qa27_refusal_0001",
        "responseId（按 id 重查判据）保留：{usage}"
    );
    assert_eq!(
        usage["usage"]["inputTokens"], 100,
        "token 计数保留：{usage}"
    );
    assert!(
        usage["diagnosticSha256"].is_string(),
        "诊断资产引用保留：{usage}"
    );
    assert!(
        manual.count("POST", "/v1/responses") >= 1,
        "说明书 AI fixture 必须被调用（用例前提）"
    );
}

/// BUG-009 覆盖性核对（形态 3 · 说明书 AI 提供方文本，**展示侧**）：即使提供方文本
/// 内嵌签名 URL，任务详情响应也不得出现 URL 形态/签名（当前实现已兜住）。
#[tokio::test]
async fn qa_bug009_provider_text_url_never_shows_in_job_detail() {
    use axum::http::{Method, StatusCode};

    const CANARY: &str = "qa27-manual-canary-5e42";
    let (app, _pool, job_id, cookie, manual) =
        qa27_run_manual_refusal("qa27-bug009-manual-dto", CANARY).await;

    let detail = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let body = detail.json();
    qa27_assert_clean(&body.to_string(), CANARY, "任务详情响应体");
    // 缺项消息（展示侧）保留可诊断的摘要标签与结论文案。
    let needs_input = body["data"]["stages"]
        .as_array()
        .expect("stages 数组")
        .iter()
        .find(|stage| stage["stageKind"] == "manual_extract")
        .expect("manual_extract 阶段")["needsInput"]
        .clone();
    assert!(
        needs_input.to_string().contains("临时供应商地址已脱敏"),
        "缺项消息应保留摘要标签：{needs_input}"
    );
    assert!(
        needs_input.to_string().contains("该批不产生正式知识"),
        "结论文案应保留：{needs_input}"
    );
    assert!(
        manual.count("POST", "/v1/responses") >= 1,
        "说明书 AI fixture 必须被调用（用例前提）"
    );
}

/// BUG-009 回归（形态 2 · HTTP 报错 + 续签）：403（链接过期）→ 按 task_id 免费重查
/// 取新链接 → 500。两条链接的签名都不得落库/出详情，付费提交恒为 1 次。
#[tokio::test]
async fn qa_bug009_http_error_path_after_refresh_never_persists_signed_url() {
    use axum::http::{Method, StatusCode};

    const STALE: &str = "qa27-canary-stale-9d2e";
    const FRESH: &str = "qa27-canary-fresh-1c07";

    let cdn = QaServer::start(vec![
        qa_repeat_rule("GET", "/m-old.glb", vec![qa_respond(403, &[], b"expired")]),
        qa_repeat_rule("GET", "/m-new.glb", vec![qa_respond(500, &[], b"boom")]),
    ]);
    let stale_url = cdn.url(&format!("/m-old.glb?sign={STALE}"));
    let fresh_url = cdn.url(&format!("/m-new.glb?sign={FRESH}"));
    let mut task_rule = qa_prefix_rule(
        "GET",
        "/v3/tasks/",
        vec![
            qa_json_respond(&qa_task_success(&stale_url)),
            qa_json_respond(&qa_task_success(&fresh_url)),
        ],
    );
    task_rule.repeat_last = true;
    let api = QaServer::start(vec![
        qa_repeat_rule("POST", "/v3/files", qa_upload_steps()),
        qa_submit_rule(),
        task_rule,
    ]);
    let api_base = format!("http://127.0.0.1:{}/v3", api.port());

    let (app, cookie, csrf) = qa_app("qa27-bug009-http", &api_base).await;
    let (job_id, _item_id) = qa_create_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock), None);
    let pool = app.state().database().pool().clone();

    let stage = qa_tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::RetryWait,
        90,
    )
    .await;
    let last_error = stage.last_error.clone().unwrap_or_default();
    println!("QA27 http last_error = {last_error}");
    assert!(
        last_error.contains("HTTP 500"),
        "HTTP 状态码必须保留（诊断能力）：{last_error}"
    );
    assert!(
        last_error.contains("下载可安全重试"),
        "结论必须保留：{last_error}"
    );
    qa27_assert_clean(&last_error, STALE, "HTTP 错误的 last_error");
    qa27_assert_clean(&last_error, FRESH, "HTTP 错误的 last_error");

    // 两条链接都被尝试（过期 1 次 → 续签拿新链接 1 次）。
    assert_eq!(cdn.count("GET", "/m-old.glb"), 1, "过期链接只尝试 1 次");
    assert_eq!(cdn.count("GET", "/m-new.glb"), 1, "续签后的新链接被尝试");
    // 恢复语义：重新查询走免费 GET；付费提交恒为 1 次（不重新购买）。
    assert!(
        api.requests()
            .iter()
            .filter(|request| request.path.starts_with("/v3/tasks/"))
            .count()
            >= 2,
        "链接过期后必须按 task_id 重新查询"
    );
    assert_eq!(
        api.count("POST", "/v3/generation/multiview-to-model"),
        1,
        "不得重新购买"
    );
    // CDN 请求不带凭据（Authorization 只发给供应商 API）。
    for request in cdn.requests() {
        assert!(
            request.header("authorization").is_none(),
            "CDN 请求不得带 Authorization"
        );
    }

    // 两条链接的签名都不得落库到任何表/列。
    for canary in [STALE, FRESH] {
        let hits = qa_canary_hits(&pool, canary).await;
        assert!(hits.is_empty(), "{canary} 落库：{hits:?}");
    }

    // 任务详情 DTO：可诊断信息（host 摘要、状态码、task ID 结论）保留，签名不出现。
    let detail = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let body = detail.json();
    let body_text = body.to_string();
    qa27_assert_clean(&body_text, STALE, "任务详情响应体");
    qa27_assert_clean(&body_text, FRESH, "任务详情响应体");
    let dto_error = body["data"]["stages"]
        .as_array()
        .expect("stages 数组")
        .iter()
        .find(|stage| stage["stageKind"] == "model_download")
        .and_then(|stage| stage["lastError"].as_str())
        .expect("model_download.lastError 非空")
        .to_owned();
    println!("QA27 dto lastError = {dto_error}");
    assert!(dto_error.contains("HTTP 500"), "{dto_error}");
    // 诊断能力不退化：轮询事实里的模型下载 host 摘要仍在（task_id 与摘要保留）。
    let poll_stage = body["data"]["stages"]
        .as_array()
        .expect("stages 数组")
        .iter()
        .find(|stage| stage["stageKind"] == "tripo_poll")
        .expect("tripo_poll 阶段");
    let poll_usage = poll_stage["usage"].to_string();
    assert!(
        poll_usage.contains("redacted"),
        "轮询事实保留脱敏摘要（诊断）：{poll_usage}"
    );
    assert!(
        poll_usage.contains("\"host\":\"127.0.0.1\""),
        "轮询事实保留 host 摘要：{poll_usage}"
    );
}
