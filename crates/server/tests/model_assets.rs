//! T13 集成测试：**模型下载、校验与不可变版本**（PRD 修订 2 / ui_revision 2；REQ-028 主，
//! AC-043、AC-044）。
//!
//! 覆盖（命令 ↔ AC 见 implementation.md §T13）：
//! - **GLB 结构校验**（contracts.md §7）：magic/version/声明长度/chunk 长度/JSON 结构/
//!   bufferView-accessor 范围/索引边界/有限坐标/非空几何/内嵌资源/required extension/
//!   面数与贴图预算；
//! - **下载安全**（architecture.md §7）：独立无凭据 client（**不带 Authorization**）、
//!   HTTPS + 允许域、逐跳重定向校验、拒绝私网/回环/链路本地、DNS 重绑定（注入解析器）、
//!   大小上限、磁盘满、不半提交；
//! - **结果处理**：流式 sha256 → 内容寻址落盘 → asset(`purpose=model`) → 不可变
//!   `model_revision`（validated）；超预算/结构问题 → `needs_input` + `rejected`
//!   revision（**原始模型与错误保留**，不静默改坏模型、不自动降预算）；
//! - **失败语义**：链接过期 → 重新查询已知任务取新链接（付费提交计数不增加）；
//!   产物过期 → 不可找回且不重购；下载中断可安全重试（整文件重下）；本地副本优先；
//! - **临时 URL 不作为永久地址**：`assets`/`model_revisions`/下载阶段 usage 都不含签名 URL。
//!
//! 隔离与门控：全部 HTTP 指向 T05 的本机 fixture（`127.0.0.1:随机端口`）；模型下载的
//! 本机放行需要**测试构建开关 + 显式测试配置**（`download.allow_local_fixture = true`）
//! 两道门；私网/链路本地地址在任何配置下都拒绝。真实收费调用只在 T23（AC-042），
//! 本文件不产生任何付费请求（付费 POST 计数断言）。

mod common;

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir};
use everything_manual::assets::blob_store::SpaceProbe;
use everything_manual::assets::glb::{
    DownloadError, DownloadPolicy, GlbBudget, GlbError, HostResolver, ModelDownloader,
    ResolveFuture, inspect,
};
use everything_manual::config::{ProviderSettings, SecretString};
use everything_manual::generation::catalog;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{
    ExecutorConfig, JobExecutor, ManualClock, StageRegistry, TickOutcome,
};
use everything_manual::providers::tripo::TripoHandlers;
use everything_manual::storage::repo::{
    job_stages as stages_repo, model_revisions as revisions_repo,
};
use manual_core::domain::{JobStage, JobStatus, ModelValidationState, StageKind};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_support::generate::build_glb;
use test_support::scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step};
use test_support::{FixtureServer, sha256_hex};

const PASSWORD: &str = "test-password-t13-model-3f81";
/// 测试用假凭据（canary）：断言不得出现在 CDN 请求头与阶段事实里。
const CANARY_KEY: &str = "canary-t13-not-a-real-key";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";
/// 供应商任务 ID（与 T05 fixture 同形）。
const TASK_ID: &str = "fixture-task-0001";

const TEST_CATALOG: &str = r#"
version = "2026-09-11"
snapshot_date = "2026-09-11"

[[tripo.presets]]
preset = "tripo-h-v3.1-standard"
model = "v3.1-20260211"
credits = "30"

[manual_ai.models.gpt-5-mini]
input_usd_per_million_tokens = "0.25"
output_usd_per_million_tokens = "2.00"
image_usd_per_image = "0.01"
"#;

// ---------------------------------------------------------------------------
// fixture / 资产工具
// ---------------------------------------------------------------------------

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name))
        .unwrap_or_else(|error| panic!("读取样例资产 {name} 失败：{error}"))
}

/// 解析样例 GLB → `(JSON, BIN)`（测试用最小解析，只支持 JSON + BIN 两个 chunk）。
fn split_glb(glb: &[u8]) -> (Value, Vec<u8>) {
    assert_eq!(&glb[0..4], b"glTF", "样例必须是 GLB");
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    let json: Value = serde_json::from_slice(&glb[20..20 + json_len]).expect("样例 JSON 可解析");
    let bin_header = 20 + json_len;
    let bin_len = u32::from_le_bytes(glb[bin_header..bin_header + 4].try_into().unwrap()) as usize;
    assert_eq!(&glb[bin_header + 4..bin_header + 8], b"BIN\0");
    let bin = glb[bin_header + 8..bin_header + 8 + bin_len].to_vec();
    (json, bin)
}

/// 用修改后的 JSON 重建 GLB（chunk 长度与总长度都重算，4 字节对齐）。
fn rebuild_glb(json: &mut Value, bin: &[u8]) -> Vec<u8> {
    let mut json_bytes = serde_json::to_vec(json).expect("序列化 glTF JSON");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin_padded = bin.to_vec();
    while !bin_padded.len().is_multiple_of(4) {
        bin_padded.push(0);
    }
    if let Some(buffers) = json.get_mut("buffers").and_then(Value::as_array_mut)
        && let Some(buffer) = buffers.first_mut()
    {
        buffer["byteLength"] = json!(bin_padded.len());
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin_padded.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin_padded);
    glb
}

/// 以样例 GLB 为底，修改 JSON 后重建（byteLength 随 BIN 重算）。
fn glb_with_json(mutator: impl FnOnce(&mut Value)) -> Vec<u8> {
    let (mut json, bin) = split_glb(&build_glb());
    mutator(&mut json);
    rebuild_glb(&mut json, &bin)
}

/// 校验一段内存中的 GLB 字节（等价 `inspect_glb_file`；`len` 取实际长度）。
fn inspect_bytes(
    bytes: &[u8],
    budget: &GlbBudget,
) -> Result<everything_manual::assets::glb::GlbSummary, GlbError> {
    let mut cursor = std::io::Cursor::new(bytes.to_vec());
    inspect(&mut cursor, bytes.len() as u64, budget)
}

/// 把字节写进测试临时目录并返回绝对路径（fixture 用 `BodySpec::File` 提供）。
fn write_temp_model(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|error| panic!("写入 {name} 失败：{error}"));
    path.to_string_lossy().into_owned()
}

fn respond_file(path: &str) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status: 200,
            headers: BTreeMap::new(),
            body: BodySpec::File {
                file: path.to_owned(),
            },
        },
    }
}

fn respond_status(status: u16) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status,
            headers: BTreeMap::new(),
            body: BodySpec::Json { json: json!({}) },
        },
    }
}

fn respond_redirect(location: &str) -> Step {
    let mut headers = BTreeMap::new();
    headers.insert("location".to_owned(), location.to_owned());
    Step::Respond {
        response: ResponseSpec {
            status: 302,
            headers,
            body: BodySpec::Text {
                text: String::new(),
            },
        },
    }
}

fn respond_json(value: Value) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status: 200,
            headers: BTreeMap::new(),
            body: BodySpec::Json { json: value },
        },
    }
}

fn exact_route(method: &str, path: &str, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Exact,
        repeat_last: false,
        steps,
    }
}

fn prefix_route(method: &str, path: &str, repeat_last: bool, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Prefix,
        repeat_last,
        steps,
    }
}

fn scenario(routes: Vec<RouteScript>) -> Scenario {
    Scenario::new(routes)
}

/// 已配置好的下载策略（本机 fixture：回环 + 测试构建两道门都满足）。
fn fixture_policy(host: &str, max_bytes: u64) -> DownloadPolicy {
    DownloadPolicy {
        allowed_hosts: vec![host.to_owned()],
        allow_local_fixture: true,
        max_bytes,
        max_redirects: 5,
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(30),
    }
}

/// 静态解析器（DNS 重绑定模拟：允许域解析到私网/任意地址）。
struct StaticResolver {
    entries: Vec<(String, Vec<IpAddr>)>,
}

impl StaticResolver {
    fn new(entries: &[(&str, &[&str])]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(host, addresses)| {
                    (
                        (*host).to_owned(),
                        addresses
                            .iter()
                            .map(|ip| ip.parse::<IpAddr>().expect("测试 IP"))
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl HostResolver for StaticResolver {
    fn resolve<'a>(&'a self, host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let result = self
            .entries
            .iter()
            .find(|(name, _)| name == host)
            .map(|(_, addresses)| addresses.clone())
            .ok_or_else(|| DownloadError::ResolutionFailed {
                host: host.to_owned(),
                detail: "测试解析器没有该主机".to_owned(),
            });
        Box::pin(std::future::ready(result))
    }
}

fn test_download_dir(tag: &str) -> TestDir {
    TestDir::new(tag)
}

/// 断言磁盘上没有下载残留（tmp 无 `.part`、blobs 下文件数符合预期）。
fn assert_no_tmp_leftovers(data_dir: &Path) {
    let tmp = data_dir.join("tmp");
    let leftovers: Vec<String> = std::fs::read_dir(&tmp)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".part"))
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "tmp 残留：{leftovers:?}");
}

fn blob_files(data_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let blobs = data_dir.join("blobs");
    let Ok(prefixes) = std::fs::read_dir(&blobs) else {
        return out;
    };
    for prefix in prefixes.filter_map(Result::ok) {
        if let Ok(files) = std::fs::read_dir(prefix.path()) {
            for file in files.filter_map(Result::ok) {
                out.push(file.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// A. GLB 结构校验（contracts.md §7 清单）
// ---------------------------------------------------------------------------

/// 正常样例：12 三角面、16px 内嵌 PNG、bounds 在 [-1,1]。
#[test]
fn valid_sample_glb_passes_all_structural_checks() {
    let bytes = build_glb();
    let summary = inspect_bytes(&bytes, &GlbBudget::default()).expect("样例必须通过");
    assert_eq!(summary.triangles, 12);
    assert_eq!(summary.vertices, 24);
    assert_eq!(summary.primitives, 1);
    assert_eq!(summary.images, 1);
    assert_eq!(summary.max_texture_dimension, 16);
    for axis in 0..3 {
        assert!(summary.bounds_min[axis] >= -1.0 && summary.bounds_max[axis] <= 1.0);
    }
    let bounds = summary.bounds_json();
    assert_eq!(bounds["triangles"], 12);
    assert!(bounds["min"].as_array().is_some());
}

#[test]
fn wrong_magic_and_truncated_container_are_rejected() {
    let mut bytes = build_glb();
    bytes[0] = b'G';
    let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "glb_magic");

    let mut bytes = build_glb();
    bytes[4] = 3; // version=3
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "glb_version"
    );

    // 声明长度不符（文件被截断）。
    let mut bytes = build_glb();
    let declared = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    bytes[8..12].copy_from_slice(&(declared + 4).to_le_bytes());
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "glb_declared_length"
    );

    // 直接截断文件（内容缺失）。
    let bytes = build_glb();
    let truncated = &bytes[..bytes.len() - 16];
    let error = inspect_bytes(truncated, &GlbBudget::default()).unwrap_err();
    assert!(
        matches!(error.code(), "glb_declared_length" | "glb_chunk_layout"),
        "{error:?}"
    );
}

#[test]
fn chunk_length_mismatch_and_json_errors_are_rejected() {
    // JSON chunk 长度字段被改坏（与文件实际布局不符）。
    let mut bytes = build_glb();
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    bytes[12..16].copy_from_slice(&(json_len + 8).to_le_bytes());
    let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "glb_chunk_layout", "{error:?}");

    // JSON 不可解析。
    let (_, bin) = split_glb(&build_glb());
    let mut json_bytes = b"{not json".to_vec();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    let total = 12 + 8 + json_bytes.len();
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    let error = inspect_bytes(&glb, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "glb_json", "{error:?}");
    assert!(!bin.is_empty(), "样例 BIN 非空");
}

#[test]
fn accessor_out_of_range_and_bad_asset_version_are_rejected() {
    // accessor 声明的 count 超出 bufferView 范围。
    let bytes = glb_with_json(|json| {
        json["accessors"][0]["count"] = json!(100_000);
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_accessor_range"
    );

    // bufferView 超出 buffer 长度。
    let bytes = glb_with_json(|json| {
        json["bufferViews"][0]["byteLength"] = json!(999_999_999);
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_buffer_range"
    );

    let bytes = glb_with_json(|json| {
        json["asset"]["version"] = json!("1.0");
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_asset_version"
    );
}

#[test]
fn non_finite_positions_and_out_of_range_indices_are_rejected() {
    // 把第一个 POSITION 的 x 改成 NaN（浮点位模式 0x7FC00000）。
    let (mut json, mut bin) = split_glb(&build_glb());
    bin[0..4].copy_from_slice(&0x7FC0_0000_u32.to_le_bytes());
    let bytes = rebuild_glb(&mut json, &bin);
    let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "gltf_non_finite_position", "{error:?}");

    // 把第一个索引改成越界值（顶点数 24）。
    let (mut json, bin) = split_glb(&build_glb());
    let index_view = json["accessors"][3]["bufferView"].as_u64().unwrap() as usize;
    let index_offset = json["bufferViews"][index_view]["byteOffset"]
        .as_u64()
        .unwrap() as usize;
    let mut bin = bin;
    bin[index_offset..index_offset + 2].copy_from_slice(&999_u16.to_le_bytes());
    let bytes = rebuild_glb(&mut json, &bin);
    let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "gltf_index_out_of_range", "{error:?}");
}

#[test]
fn empty_geometry_and_missing_position_are_rejected() {
    let bytes = glb_with_json(|json| {
        json["meshes"][0]["primitives"] = json!([]);
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_empty_geometry"
    );

    let bytes = glb_with_json(|json| {
        json["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("POSITION");
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_missing_position"
    );

    // 线框（mode=1）不产生三角面 → 空几何。
    let bytes = glb_with_json(|json| {
        json["meshes"][0]["primitives"][0]["mode"] = json!(1);
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_empty_geometry"
    );
}

#[test]
fn external_uris_and_unsupported_features_are_rejected() {
    // 外链 buffer（含 data: URI 也拒绝：首版只接受 BIN chunk）。
    for uri in [
        "https://evil.example/big.bin",
        "data:application/octet-stream;base64,AAAA",
    ] {
        let bytes = glb_with_json(|json| {
            json["buffers"][0]["uri"] = json!(uri);
        });
        let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
        assert_eq!(error.code(), "gltf_external_uri", "{uri}：{error:?}");
    }

    // 外链贴图。
    let bytes = glb_with_json(|json| {
        json["images"][0]["uri"] = json!("https://evil.example/tex.png");
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_external_uri"
    );

    // 未支持的 required extension（例如 Draco 压缩）。
    let bytes = glb_with_json(|json| {
        json["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    });
    let error = inspect_bytes(&bytes, &GlbBudget::default()).unwrap_err();
    assert_eq!(error.code(), "gltf_required_extension");
    assert!(
        error.message().contains("KHR_draco_mesh_compression"),
        "{error:?}"
    );

    // sparse accessor：明确拒绝（不是静默忽略）。
    let bytes = glb_with_json(|json| {
        json["accessors"][0]["sparse"] = json!({"count": 1, "indices": {}, "values": {}});
    });
    assert_eq!(
        inspect_bytes(&bytes, &GlbBudget::default())
            .unwrap_err()
            .code(),
        "gltf_unsupported_feature"
    );
}

#[test]
fn face_and_texture_budgets_are_enforced_when_over() {
    let bytes = build_glb();
    let budget = GlbBudget {
        max_triangles: 4,
        max_texture_dimension: 4096,
    };
    let error = inspect_bytes(&bytes, &budget).unwrap_err();
    assert_eq!(error.code(), "gltf_face_limit");
    assert!(error.is_budget());
    assert!(error.message().contains("不自动降面数"), "{error:?}");

    let budget = GlbBudget {
        max_triangles: 100_000,
        max_texture_dimension: 8,
    };
    let error = inspect_bytes(&bytes, &budget).unwrap_err();
    assert_eq!(error.code(), "gltf_texture_limit");
    assert!(error.is_budget());

    // 默认预算（100000 面 / 4096 px）下样例通过。
    assert!(inspect_bytes(&bytes, &GlbBudget::default()).is_ok());
}

// ---------------------------------------------------------------------------
// B. 下载层（SSRF 防护、大小、磁盘、重定向）
// ---------------------------------------------------------------------------

/// 正常下载：流式 sha256 一致、内容寻址落盘、**请求头不含 Authorization**、
/// 只有一次请求（没有多余探测）。
#[tokio::test]
async fn download_streams_to_blob_without_forwarding_credentials() {
    let glb = build_glb();
    let expected = sha256_hex(&glb);
    let server = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let dir = test_download_dir("t13-download-ok");
    let policy = fixture_policy("127.0.0.1", 150 * 1_048_576);
    let downloader = ModelDownloader::new(dir.path(), policy);
    let url = format!("{}/model.glb?sign=secret-signature", server.base_url());

    let downloaded = downloader.download(&url).await.expect("下载成功");
    assert_eq!(downloaded.sha256, expected);
    assert_eq!(downloaded.size, glb.len() as u64);
    assert!(downloaded.path.is_file(), "blob 文件必须落盘");
    assert_eq!(
        std::fs::read(&downloaded.path).unwrap(),
        glb,
        "落盘字节必须与响应字节一致"
    );
    assert_eq!(blob_files(dir.path()), vec![expected.clone()]);
    assert_no_tmp_leftovers(dir.path());

    // 独立无凭据 client：CDN 请求不得带 Authorization（也不带其它凭据头）。
    server.assert_called_times("GET", "/model.glb", 1);
    let records = server.requests_matching("GET", "/model.glb");
    for record in &records {
        let names: Vec<String> = record
            .headers
            .iter()
            .map(|header| header.name.to_ascii_lowercase())
            .collect();
        assert!(
            !names.iter().any(|name| name == "authorization"),
            "模型 CDN 请求不得带 Authorization：{names:?}"
        );
        assert!(!names.iter().any(|name| name == "cookie"), "{names:?}");
        assert!(!names.iter().any(|name| name == "x-api-key"), "{names:?}");
    }
    server.assert_no_script_problems();
}

/// 允许域与 HTTPS：名单为空/未命中 → 拒绝；生产策略下 http 与回环都被拒。
#[tokio::test]
async fn download_rejects_disallowed_hosts_and_insecure_schemes() {
    let server = FixtureServer::start(scenario(vec![]));
    let dir = test_download_dir("t13-download-policy");
    let url = format!("{}/model.glb", server.base_url());

    // 空名单 = 拒绝一切下载（不猜测 CDN 域名；用 https 避免先命中明文 http 检查）。
    let downloader = ModelDownloader::new(dir.path(), DownloadPolicy::default());
    let error = downloader
        .download("https://127.0.0.1:9/model.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_host_not_allowed");
    // 同一策略下明文 http 也被拒（scheme 检查先于允许域检查）。
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_insecure_scheme");

    // 生产策略（无本机 fixture 放行）：明文 http → 拒绝。
    let mut policy = fixture_policy("127.0.0.1", 1024);
    policy.allow_local_fixture = false;
    let downloader = ModelDownloader::new(dir.path(), policy);
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_insecure_scheme");

    // 生产策略：https + 回环地址 → 拒绝（SSRF 防护，私网/回环不访问）。
    let mut policy = fixture_policy("127.0.0.1", 1024);
    policy.allow_local_fixture = false;
    let downloader = ModelDownloader::new(dir.path(), policy);
    let error = downloader
        .download("https://127.0.0.1:9/model.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert!(error.message().contains("回环地址"), "{error:?}");

    // 私网 URL 即使开启本机 fixture 放行也拒绝。
    let downloader = ModelDownloader::new(dir.path(), fixture_policy("10.1.2.3", 1024));
    let error = downloader
        .download("http://10.1.2.3/model.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");

    assert_eq!(server.request_total(), 0, "以上拒绝都不应发出任何请求");
}

/// DNS 重绑定：允许域解析到私网地址 → 拒绝；解析到本机（测试放行）→ 允许。
#[tokio::test]
async fn dns_rebinding_to_private_address_is_rejected() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let dir = test_download_dir("t13-dns-rebinding");
    let policy = fixture_policy("cdn.allowed.invalid", 150 * 1_048_576);

    // 重绑定：允许域解析到 10.1.2.3（私网）→ 拒绝，且不发请求。
    let downloader = ModelDownloader::new(dir.path(), policy.clone()).with_resolver(Arc::new(
        StaticResolver::new(&[("cdn.allowed.invalid", &["10.1.2.3"])]),
    ));
    let error = downloader
        .download("http://cdn.allowed.invalid/model.glb?sign=x")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert_eq!(server.request_total(), 0, "重绑定目标不得被连接");

    // 混合答案（一个公网 + 一个私网）整体拒绝。
    let downloader = ModelDownloader::new(dir.path(), policy.clone()).with_resolver(Arc::new(
        StaticResolver::new(&[("cdn.allowed.invalid", &["93.184.216.34", "192.168.1.9"])]),
    ));
    let error = downloader
        .download("http://cdn.allowed.invalid/model.glb")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    assert_eq!(server.request_total(), 0);

    // 解析器把允许域映射到本机 fixture（测试构建 + 显式放行）→ 连接 pin 到该 IP 成功。
    let downloader = ModelDownloader::new(dir.path(), policy).with_resolver(Arc::new(
        StaticResolver::new(&[("cdn.allowed.invalid", &["127.0.0.1"])]),
    ));
    let url = format!(
        "http://cdn.allowed.invalid:{}/model.glb",
        server.addr().port()
    );
    let downloaded = downloader
        .download(&url)
        .await
        .expect("pin 到已校验 IP 成功");
    assert_eq!(downloaded.sha256, sha256_hex(&build_glb()));
    server.assert_called_times("GET", "/model.glb", 1);
    let record = &server.requests_matching("GET", "/model.glb")[0];
    assert_eq!(
        record.target, "/model.glb",
        "请求路径保持原样（host 由 URL 决定）"
    );
}

/// 重定向：每一跳都重新校验（私网/回环目标与非允许域都拒绝）；不转发凭据。
#[tokio::test]
async fn redirects_are_validated_hop_by_hop() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "GET",
            "/hop1.glb",
            vec![respond_redirect("http://evil.invalid/x.glb")],
        ),
        exact_route(
            "GET",
            "/hop2.glb",
            vec![respond_redirect("http://other.invalid/private.glb")],
        ),
        exact_route(
            "GET",
            "/final.glb",
            vec![respond_file("assets/sample-model.glb")],
        ),
        exact_route("GET", "/hop3.glb", vec![respond_redirect("/final.glb")]),
    ]));
    let dir = test_download_dir("t13-redirects");

    // 1) 重定向到"允许域但解析为私网"的主机 → 拒绝（连接被拦）。
    let mut policy = fixture_policy("127.0.0.1", 150 * 1_048_576);
    policy.allowed_hosts.push("evil.invalid".to_owned());
    let downloader = ModelDownloader::new(dir.path(), policy.clone()).with_resolver(Arc::new(
        StaticResolver::new(&[("evil.invalid", &["10.9.9.9"])]),
    ));
    let url = format!("{}/hop1.glb", server.base_url());
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_forbidden_address");
    server.assert_called_times("GET", "/hop1.glb", 1);
    assert_eq!(server.request_total(), 1, "被拒目标不得被连接");

    // 2) 重定向到不在允许域名单内的主机 → 拒绝。
    let downloader = ModelDownloader::new(dir.path(), policy.clone());
    let url = format!("{}/hop2.glb", server.base_url());
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_host_not_allowed");

    // 3) 相对 Location 的合法重定向 → 跟随（每一跳仍校验，都被允许）。
    let downloader = ModelDownloader::new(dir.path(), policy);
    let url = format!("{}/hop3.glb", server.base_url());
    let downloaded = downloader.download(&url).await.expect("相对重定向可跟随");
    assert_eq!(downloaded.sha256, sha256_hex(&build_glb()));
    server.assert_called_times("GET", "/hop3.glb", 1);
    server.assert_called_times("GET", "/final.glb", 1);
    server.assert_no_script_problems();
}

/// 大小上限与磁盘满：明确错误、不半提交（无 blob、无 tmp 残留）。
#[tokio::test]
async fn size_limit_and_disk_full_leave_no_partial_artifact() {
    let server = FixtureServer::start(scenario(vec![prefix_route(
        "GET",
        "/model.glb",
        true,
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let dir = test_download_dir("t13-size-disk");
    let url = format!("{}/model.glb", server.base_url());

    // 上限小于响应体（声明 Content-Length 先拦）。
    let downloader = ModelDownloader::new(dir.path(), fixture_policy("127.0.0.1", 128));
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_too_large");
    assert!(blob_files(dir.path()).is_empty());
    assert_no_tmp_leftovers(dir.path());

    // 磁盘满（注入空间探测）：明确错误、不半提交。
    let downloader = ModelDownloader::new(dir.path(), fixture_policy("127.0.0.1", 150 * 1_048_576))
        .with_space_probe(SpaceProbe::Fixed(0));
    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_insufficient_storage");
    assert!(error.message().contains("未保存任何资产"), "{error:?}");
    assert!(blob_files(dir.path()).is_empty());
    assert_no_tmp_leftovers(dir.path());
    server.assert_no_script_problems();
}

/// 下载中断（半关闭截断）：可安全重试（整文件重下，不用 Range 续传），无残留。
#[tokio::test]
async fn interrupted_download_is_safely_retryable_without_partial_files() {
    let glb = build_glb();
    let server = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![
            Step::HalfClose {
                response: ResponseSpec {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: BodySpec::File {
                        file: "assets/sample-model.glb".to_owned(),
                    },
                },
                truncate_at: glb.len() / 2,
            },
            respond_file("assets/sample-model.glb"),
        ],
    )]));
    let dir = test_download_dir("t13-interrupted");
    let downloader = ModelDownloader::new(dir.path(), fixture_policy("127.0.0.1", 150 * 1_048_576));
    let url = format!("{}/model.glb", server.base_url());

    let error = downloader.download(&url).await.unwrap_err();
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert!(blob_files(dir.path()).is_empty(), "中断不得留下 blob");
    assert_no_tmp_leftovers(dir.path());

    // 重试：重新完整下载（策略：不使用 Range 续传，避免把 sha 计算变得复杂）。
    let downloaded = downloader.download(&url).await.expect("重试成功");
    assert_eq!(downloaded.sha256, sha256_hex(&glb));
    server.assert_called_times("GET", "/model.glb", 2);
    let second = &server.requests_matching("GET", "/model.glb")[1];
    assert!(
        !second
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("range")),
        "重试为整文件重下（不发送 Range 头）"
    );
    server.assert_no_script_problems();
}

/// BUG-009（QA 回合 26）：传输类失败的**阶段消息 / 日志摘要 / Display** 都不得包含
/// 供应商签名 URL 或 `://` 形态文本；诊断语义（错误码、可安全重试）必须保留。
///
/// 复现形态：连接被拒（端口先占后放）与请求超时（黑障本机服务器）。
/// 判定与 QA 用例 `qa_t20_bug008_independent.rs` 同源，但本用例同时断言日志摘要
/// （`model_download_failed` 的 `detail` 字段就是 `log_summary()`）。
#[tokio::test]
async fn transport_failures_never_leak_signed_url_into_message_or_log() {
    const CANARY: &str = "rd-r28-canary-transport-7d31";
    const MAX_BYTES: u64 = 8 * 1024 * 1024;

    // 1) 连接被拒（生产：CDN 端口不可达/TCP RST）。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("占位端口");
    let port = listener.local_addr().expect("本地地址").port();
    drop(listener);
    let dir = test_download_dir("r28-transport-refused");
    let downloader = ModelDownloader::new(dir.path(), fixture_policy("127.0.0.1", MAX_BYTES));
    let url = format!("http://127.0.0.1:{port}/r28/model.glb?sign={CANARY}&expires=9999999999");
    let error = downloader
        .download(&url)
        .await
        .expect_err("连接被拒必须失败");
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert!(error.is_retryable(), "传输失败可安全重试：{error:?}");
    assert_signed_url_absent(&error.message(), CANARY, "阶段消息");
    assert_signed_url_absent(&error.log_summary(), CANARY, "日志摘要");
    assert_signed_url_absent(&error.to_string(), CANARY, "Display");
    assert!(
        error.message().contains("可安全重试"),
        "结论保留（下载可安全重试；链接过期按 task_id 重查）：{}",
        error.message()
    );

    // 2) 请求超时（生产：CDN 无响应；`DownloadPolicy` 默认 request 600s）。
    let server = BlackHoleServer::start();
    let mut policy = fixture_policy("127.0.0.1", MAX_BYTES);
    policy.connect_timeout = Duration::from_secs(1);
    policy.request_timeout = Duration::from_secs(1);
    let dir = test_download_dir("r28-transport-timeout");
    let downloader = ModelDownloader::new(dir.path(), policy);
    let url = format!(
        "http://127.0.0.1:{}/r28/model.glb?sign={CANARY}",
        server.port()
    );
    let error = downloader.download(&url).await.expect_err("超时必须失败");
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert_signed_url_absent(&error.message(), CANARY, "超时阶段消息");
    assert_signed_url_absent(&error.log_summary(), CANARY, "超时日志摘要");
}

fn assert_signed_url_absent(text: &str, canary: &str, context: &str) {
    assert!(
        !text.contains("://"),
        "{context} 不得含 URL 形态文本：{text}"
    );
    assert!(!text.contains(canary), "{context} 不得含签名：{text}");
    assert!(
        !text.contains("for url"),
        "{context} 不得携带 reqwest 的 URL 后缀：{text}"
    );
}

/// 黑障本机"CDN"：收下连接与请求后不写任何响应字节（触发请求超时）。
struct BlackHoleServer {
    port: u16,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BlackHoleServer {
    fn start() -> Self {
        use std::io::Read;
        use std::sync::atomic::Ordering;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定回环端口");
        listener.set_nonblocking(true).expect("非阻塞监听");
        let port = listener.local_addr().expect("本地地址").port();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut scratch = [0_u8; 4096];
                        let _ = stream.read(&mut scratch);
                        std::thread::sleep(Duration::from_secs(10));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            stop,
            thread: Some(thread),
        }
    }

    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for BlackHoleServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---------------------------------------------------------------------------
// C. 执行器端到端（下载 → 校验 → 不可变 revision）
// ---------------------------------------------------------------------------

/// 带模型 fixture 的应用（真实 API + 真实执行器；下载策略显式允许本机 fixture）。
async fn model_app(tag: &str, base_url: &str) -> (TestApp, String, String) {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    let mut tripo = common::configured_tripo(CANARY_KEY);
    tripo.base_url = base_url.to_owned();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: "https://api.openai.com/v1".to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new("canary-manual-ai-key")),
        key_source: Some("测试注入".to_owned()),
    };
    settings.download.allowed_hosts = vec!["127.0.0.1".to_owned()];
    settings.download.allow_local_fixture = true;
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), TEST_CATALOG).expect("写入价格目录");
    settings.price_catalog = Some(catalog::parse(TEST_CATALOG).expect("价格目录可解析"));
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let login = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text());
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .expect("csrfToken")
        .to_owned();
    let cookie = login.session_cookie();
    (app, cookie, csrf)
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-t13-{tag}"),
            body: Vec::new(),
        }
    }

    fn text_field(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self
    }

    fn file_field(mut self, name: &str, filename: &str, content_type: &str, bytes: &[u8]) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

struct FileSpec<'a> {
    purpose: &'a str,
    filename: &'a str,
    content_type: &'a str,
    bytes: &'a [u8],
}

async fn upload_asset(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    file: FileSpec<'_>,
) -> String {
    let (boundary, body) = Multipart::new(file.purpose)
        .text_field("purpose", file.purpose)
        .file_field("file", file.filename, file.content_type, file.bytes)
        .finish();
    let response = app
        .call(Method::POST, &format!("/api/v1/items/{item}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(Some(&boundary), body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
}

/// 走真实 API 创建一份可执行任务（与 T12 用例同形：物品 + ready 准备 + 视图 + 报价 + 建单）。
async fn create_ready_job(app: &TestApp, cookie: &str, csrf: &str) -> (String, String) {
    let item = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "T13 用例物品", "model": "X100V" }))
        .send()
        .await;
    assert_eq!(item.status, StatusCode::CREATED, "{}", item.text());
    let item_id = item.json()["data"]["id"].as_str().unwrap().to_owned();

    let pdf = fixture_bytes("sample-manual-text.pdf");
    let doc_asset = upload_asset(
        app,
        cookie,
        csrf,
        &item_id,
        FileSpec {
            purpose: "document",
            filename: "manual.pdf",
            content_type: "application/pdf",
            bytes: &pdf,
        },
    )
    .await;
    let document = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceAssetId": doc_asset, "title": "样例说明书" }))
        .send()
        .await;
    assert_eq!(document.status, StatusCode::CREATED, "{}", document.text());
    let document_id = document.json()["data"]["id"].as_str().unwrap().to_owned();
    let source_sha = document.json()["data"]["sourceSha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let created = app
        .call(
            Method::POST,
            &format!("/api/v1/documents/{document_id}/preparations"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceSha256": source_sha }))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let preparation = created.json()["data"]["id"].as_str().unwrap().to_owned();

    let page_jpeg = fixture_bytes("sample-photo-front.jpg");
    let page_text = "a".repeat(64);
    for page in 1..=2_i64 {
        let image = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "pageImage",
                filename: "page.jpg",
                content_type: "image/jpeg",
                bytes: &page_jpeg,
            },
        )
        .await;
        let text = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "pageText",
                filename: "page.txt",
                content_type: "text/plain",
                bytes: page_text.as_bytes(),
            },
        )
        .await;
        let response = app
            .call(
                Method::PUT,
                &format!("/api/v1/preparations/{preparation}/pages/{page}"),
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
        .call(Method::GET, &format!("/api/v1/preparations/{preparation}"))
        .cookie(cookie)
        .send()
        .await;
    let etag = current.header("etag").expect("准备详情 ETag");
    let completed = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", &etag)
        .json(&json!({ "pageCount": 2 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    let mut photo_ids: Vec<String> = Vec::new();
    for (view, name, content_type) in [
        ("front", "sample-photo-front.jpg", "image/jpeg"),
        ("left", "sample-photo-left.png", "image/png"),
    ] {
        let bytes = fixture_bytes(name);
        let asset = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "photo",
                filename: name,
                content_type,
                bytes: &bytes,
            },
        )
        .await;
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
            "preparationId": preparation,
            "photoIds": photo_ids,
            "modelPreset": PRESET,
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
        .header("idempotency-key", "t13-e2e-key-0001")
        .json(&json!({
            "quoteId": quote_id,
            "preparationId": preparation,
            "photoIds": photo_ids,
            "limits": { "tripoCreditMinor": 3000, "manualAiUsdMicros": 100_000 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    (
        job.json()["data"]["id"].as_str().unwrap().to_owned(),
        item_id,
    )
}

/// 按路径前缀计数（`requests_matching` 是路径完全相等）。
fn count_prefix(server: &FixtureServer, method: &str, prefix: &str) -> usize {
    server
        .requests()
        .into_iter()
        .filter(|request| {
            request.method.eq_ignore_ascii_case(method) && request.path.starts_with(prefix)
        })
        .count()
}

fn pool(app: &TestApp) -> SqlitePool {
    app.state().database().pool().clone()
}

/// 生产接线（`register_provider_handlers`）：T13 起注册 5 个 Tripo 阶段；
/// T14 起同一接线在 `providers.manual_ai` 已配置时**另注册** `manual_extract` /
/// `manual_merge`（两个 Provider 相互独立；本用例只驱动 Tripo 阶段）。
fn model_executor(app: &TestApp, clock: Arc<ManualClock>) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let mut registry = StageRegistry::new();
    let registered =
        everything_manual::providers::register_provider_handlers(&mut registry, &settings)
            .expect("已配置的 Tripo 必须能注册");
    for stage in [
        StageKind::TripoUpload,
        StageKind::TripoSubmit,
        StageKind::TripoPoll,
        StageKind::ModelDownload,
        StageKind::ModelValidate,
    ] {
        assert!(
            registered.contains(&stage),
            "T13 起下载与校验阶段必须注册（实际：{registered:?}）"
        );
        assert!(registry.contains(stage));
    }
    fixed_jitter_executor(
        pool(app),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

/// 测试注入接线：自定义下载器与 GLB 预算（覆盖磁盘满 / 超预算路径）。
fn injected_executor(
    app: &TestApp,
    clock: Arc<ManualClock>,
    downloader: ModelDownloader,
    budget: GlbBudget,
) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let handlers = TripoHandlers::from_settings(&settings)
        .expect("已配置的 Tripo 必须能构造")
        .with_download(downloader, budget);
    let mut registry = StageRegistry::new();
    handlers.register(&mut registry);
    fixed_jitter_executor(
        pool(app),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

async fn run_ticks(
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    ticks: usize,
    step_millis: i64,
) -> Vec<everything_manual::jobs::StageRunReport> {
    let mut reports = Vec::new();
    for _ in 0..ticks {
        clock.advance_millis(step_millis);
        match executor.tick().await.expect("tick") {
            TickOutcome::Executed(report) => reports.push(report),
            TickOutcome::Idle => {}
        }
    }
    reports
}

async fn stage_of(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    stages_repo::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .unwrap_or_else(|| panic!("阶段不存在：{}", kind.as_str()))
}

async fn stage_status(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStatus {
    stage_of(pool, job_id, kind).await.status
}

/// 反复 tick（每次推进 20s）直到目标阶段达到期望状态。
async fn tick_until_stage(
    pool: &SqlitePool,
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    job_id: &str,
    kind: StageKind,
    want: JobStatus,
    max_ticks: usize,
) {
    for _ in 0..max_ticks {
        if stage_status(pool, job_id, kind).await == want {
            return;
        }
        run_ticks(executor, clock, 1, 20_000).await;
    }
    let stage = stage_of(pool, job_id, kind).await;
    panic!(
        "阶段 {} 未在 {max_ticks} tick 内达到 {}（实际 {}；last_error={:?}；needs_input={:?}）",
        kind.as_str(),
        want.as_str(),
        stage.status.as_str(),
        stage.last_error,
        stage.needs_input_json
    );
}

/// Tripo 成功响应的模型 URL 指向本机 fixture（`.invalid` 域在生产用例中不可达）。
fn task_success_with(model_url: &str, extra: Value) -> Value {
    let mut data = json!({
        "task_id": TASK_ID,
        "status": "success",
        "progress": 100,
        "credits_consumed": 30,
        "output": {
            "model_url": model_url,
            "rendered_image_url": "https://cdn.example.invalid/preview.png",
        }
    });
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            data[key] = value.clone();
        }
    }
    json!({ "code": 0, "data": data })
}

fn upload_steps() -> Vec<Step> {
    vec![
        respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
        respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
    ]
}

/// 端到端：上传 → 付费提交（1 次）→ 查询 → **下载 → 校验 → 不可变 revision**。
///
/// 断言：sha256 与字节一致、blob 落盘、asset(`purpose=model`)、revision `validated` 且
/// bounds 非空、CDN 请求无 Authorization、临时签名 URL 不作为永久地址保存。
#[tokio::test]
async fn end_to_end_downloads_validates_and_creates_immutable_revision() {
    let glb = build_glb();
    let expected_sha = sha256_hex(&glb);
    // 先起"模型 CDN"（端口已知），再把 API fixture 的 model_url 指向它（同为本机回环）。
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let model_url = format!("{}/model.glb?sign=fixture-signature-secret", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) = model_app("t13-e2e", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = model_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        80,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;

    // 付费提交只发生一次（下载与校验都不产生费用）。
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
    // CDN 请求一次且不带凭据。
    cdn.assert_called_times("GET", "/model.glb", 1);
    let request = &cdn.requests_matching("GET", "/model.glb")[0];
    assert!(
        !request
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("authorization")),
        "模型 CDN 请求不得携带 API Authorization"
    );
    assert_eq!(
        request.query.as_deref(),
        Some("sign=fixture-signature-secret"),
        "签名查询串按原样发送（不落到永久地址）"
    );

    // blob 落盘 + 内容一致。
    let data_dir = app.dir();
    let blob_path = data_dir
        .join("blobs")
        .join(&expected_sha[..2])
        .join(&expected_sha);
    assert!(
        blob_path.is_file(),
        "blob 必须落盘：{}",
        blob_path.display()
    );
    assert_eq!(std::fs::read(&blob_path).unwrap(), glb);
    assert_no_tmp_leftovers(data_dir);

    // asset（purpose=model）+ 不可变 revision（validated + bounds）。
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &item_id, &expected_sha)
        .await
        .expect("读 revision")
        .expect("revision 必须存在");
    assert_eq!(revision.validation_state, ModelValidationState::Validated);
    assert_eq!(revision.sha256, expected_sha);
    assert!(
        revision.bounds.is_some(),
        "validated revision 必须有 bounds"
    );
    assert!(
        revision.provider_attempt_id.is_some(),
        "应关联付费提交 attempt"
    );
    let asset = everything_manual::storage::repo::assets::get(&mut conn, &revision.asset_id)
        .await
        .unwrap()
        .expect("asset 必须存在");
    assert_eq!(asset.purpose, manual_core::domain::AssetPurpose::Model);
    assert_eq!(asset.blob_id, expected_sha);
    assert_eq!(
        asset.original_name.as_deref(),
        Some("model.glb"),
        "原文件名只作元数据（取自 URL 路径，不含查询串）"
    );
    let download_stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    let validate_stage = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    assert_eq!(
        validate_stage.result_asset_id.as_deref(),
        Some(asset.id.as_str())
    );

    // 临时供应商 URL 不作为永久地址保存：revision/asset/下载阶段 usage 均不含签名 URL。
    let revision_row: (Option<String>,) =
        sqlx::query_as("SELECT bounds FROM model_revisions WHERE id = ?")
            .bind(&revision.id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    for text in [
        revision_row.0.unwrap_or_default(),
        download_stage
            .usage_json
            .clone()
            .unwrap_or_default()
            .to_string(),
        validate_stage
            .usage_json
            .clone()
            .unwrap_or_default()
            .to_string(),
    ] {
        assert!(!text.contains("sign=fixture-signature-secret"), "{text}");
        assert!(!text.contains("http"), "usage/bounds 不得含 URL：{text}");
    }
    let asset_row: (String, Option<String>) =
        sqlx::query_as("SELECT blob_id, original_name FROM assets WHERE id = ?")
            .bind(&asset.id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(asset_row.0, expected_sha);
    assert!(!asset_row.1.unwrap_or_default().contains("http"));

    // 校验阶段 usage 记录 revision 与结构摘要。
    let usage = validate_stage.usage_json.expect("校验阶段 usage");
    assert_eq!(usage["validation"], "validated");
    assert_eq!(usage["triangles"], 12);
    assert_eq!(usage["modelRevisionId"], revision.id);
    assert_eq!(download_stage.usage_json.unwrap()["linkRefreshed"], false);
    server.assert_no_script_problems();
}

/// 链接过期 → **重新查询已知任务**取新链接（付费提交计数不增加）。
#[tokio::test]
async fn expired_link_is_refreshed_by_requerying_the_known_task() {
    let glb = build_glb();
    let expected_sha = sha256_hex(&glb);
    let cdn = FixtureServer::start(scenario(vec![
        exact_route("GET", "/model.glb", vec![respond_status(403)]),
        exact_route(
            "GET",
            "/model-v2.glb",
            vec![respond_file("assets/sample-model.glb")],
        ),
    ]));
    let expired_url = format!("{}/model.glb?sign=expired", cdn.base_url());
    let fresh_url = format!("{}/model-v2.glb?sign=fresh", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            false,
            vec![
                respond_json(task_success_with(&expired_url, json!(null))),
                respond_json(task_success_with(&fresh_url, json!(null))),
            ],
        ),
    ]));
    let (app, cookie, csrf) = model_app("t13-refresh", &format!("{}/v3", server.base_url())).await;
    let (job_id, _item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = model_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        80,
    )
    .await;

    // 旧的（过期）链接被请求一次并得到 403 → 重新查询任务 → 新链接下载成功。
    cdn.assert_called_times("GET", "/model.glb", 1);
    cdn.assert_called_times("GET", "/model-v2.glb", 1);
    assert!(
        count_prefix(&server, "GET", "/v3/tasks/") >= 2,
        "必须重新查询已存在的远端任务（轮询 1 次 + 过期后重查 1 次）"
    );
    // **不重新购买**：付费提交仍只有一次。
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
    let download_stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    assert_eq!(download_stage.usage_json.unwrap()["linkRefreshed"], true);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &_item_id, &expected_sha)
        .await
        .unwrap()
        .expect("revision");
    assert_eq!(revision.validation_state, ModelValidationState::Validated);
    server.assert_no_script_problems();
}

/// T20/BUG-008：签名 URL 不落库；**进程重启/恢复**后下载阶段按 task ID 重新查询
/// 取新链接（免费），绝不重新购买，也不依赖库里曾保存的 URL。
#[tokio::test]
async fn restarted_download_requeries_by_task_id_and_never_repurchases() {
    let glb = build_glb();
    let expected_sha = sha256_hex(&glb);
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let observed_url = format!("{}/model.glb?sign=first-observation", cdn.base_url());
    let refreshed_url = format!("{}/model.glb?sign=after-restart", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        // 查询 1 = 轮询（第一次观察）；查询 2 = 重启后的下载阶段重新查询（续签链接）。
        prefix_route(
            "GET",
            "/v3/tasks/",
            false,
            vec![
                respond_json(task_success_with(&observed_url, json!(null))),
                respond_json(task_success_with(&refreshed_url, json!(null))),
            ],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-restart-requery", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let pool = pool(&app);

    // 第一个执行器只把链路推进到 tripo_poll 成功（同进程内它会把链接记进易失缓存）。
    let first = model_executor(&app, Arc::clone(&clock));
    tick_until_stage(
        &pool,
        &first,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let poll_stage = stage_of(&pool, &job_id, StageKind::TripoPoll).await;
    let usage = poll_stage.usage_json.clone().expect("查询事实");
    assert_eq!(usage["remoteTaskId"], TASK_ID);
    let rendered = usage.to_string();
    assert!(!rendered.contains("://"), "落库事实不得含 URL：{rendered}");
    assert!(
        !rendered.contains("sign=first-observation"),
        "落库事实不得含签名：{rendered}"
    );
    assert_eq!(usage["modelUrl"]["redacted"], true);
    assert_eq!(
        usage["modelUrl"]["sha256"],
        everything_manual::redaction::url_summary(&observed_url).sha256_prefix,
        "摘要可复核：sha256(URL) 前缀"
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ModelDownload)
            .await
            .status,
        JobStatus::Queued,
        "用例前提：第一个执行器尚未执行下载阶段"
    );

    // "重启"：新的执行器（新注册表 → 易失缓存为空）继续跑完下载与校验。
    let second = model_executor(&app, Arc::clone(&clock));
    tick_until_stage(
        &pool,
        &second,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        80,
    )
    .await;
    tick_until_stage(
        &pool,
        &second,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;

    // 重新查询过已知任务（≥ 2 次 GET /v3/tasks/），下载用的是**续签后**的链接。
    assert!(
        count_prefix(&server, "GET", "/v3/tasks/") >= 2,
        "重启后必须按 task ID 重新查询"
    );
    let targets: Vec<String> = cdn
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET")
        .map(|request| request.target)
        .collect();
    assert_eq!(targets.len(), 1, "模型只下载一次：{targets:?}");
    assert!(
        targets[0].contains("sign=after-restart"),
        "必须使用重新查询取回的新链接：{targets:?}"
    );
    // 付费提交仍只有一次（重新查询不等于重新购买）。
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);

    let download_stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    assert_eq!(
        download_stage.usage_json.clone().unwrap()["sha256"],
        expected_sha
    );
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &item_id, &expected_sha)
        .await
        .unwrap()
        .expect("revision");
    assert_eq!(revision.validation_state, ModelValidationState::Validated);
    server.assert_no_script_problems();
    assert_no_tmp_leftovers(app.dir());
}

/// 产物过期（再查显示 expired）→ 明确不可恢复、不重新购买。
#[tokio::test]
async fn expired_artifact_is_unrecoverable_without_a_new_purchase() {
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_status(403)],
    )]));
    let expired_url = format!("{}/model.glb?sign=expired", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            false,
            vec![
                respond_json(task_success_with(&expired_url, json!(null))),
                respond_json(json!({
                    "code": 0,
                    "data": { "task_id": TASK_ID, "status": "expired", "progress": 100 }
                })),
            ],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-artifact-expired", &format!("{}/v3", server.base_url())).await;
    let (job_id, _) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = model_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Failed,
        80,
    )
    .await;
    let stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    let last_error = stage.last_error.unwrap_or_default();
    assert!(last_error.contains("不可找回"), "{last_error}");
    assert!(last_error.contains("不会自动重新购买"), "{last_error}");
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
    let mut conn = pool.acquire().await.unwrap();
    let model_assets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE purpose = 'model'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(model_assets, 0, "产物不可找回时不落盘模型资产");
    assert_no_tmp_leftovers(app.dir());
}

/// 本地副本优先：已成功下载过的阶段重跑时不重复下载（链接可能已过期）。
#[tokio::test]
async fn local_copy_is_reused_when_the_stage_is_retried() {
    let glb = build_glb();
    let expected_sha = sha256_hex(&glb);
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let model_url = format!("{}/model.glb?sign=one-time", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-local-copy", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = model_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let requests_before = server.request_total() + cdn.request_total();

    // 把下载阶段重置为 queued（模拟"分支重试"）：本地副本仍在 → 不再下载。
    let stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "UPDATE job_stages SET status = 'queued', lease_owner = NULL, lease_until = NULL, \
         next_run_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&stage.id)
    .execute(&mut *pool.acquire().await.unwrap())
    .await
    .unwrap();

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::Succeeded,
        20,
    )
    .await;
    assert_eq!(
        server.request_total() + cdn.request_total(),
        requests_before,
        "本地副本存在时不得发起任何新请求（临时链接可能已过期）"
    );
    let stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    assert_eq!(stage.usage_json.unwrap()["reusedLocalCopy"], true);
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &item_id, &expected_sha)
        .await
        .unwrap()
        .expect("复用本地副本后仍能创建 revision");
    assert_eq!(revision.validation_state, ModelValidationState::Validated);
}

/// 超预算（注入小预算）→ `needs_input` + `rejected` revision；**原始模型保留**。
#[tokio::test]
async fn over_budget_model_keeps_the_original_and_enters_needs_input() {
    let glb = build_glb();
    let expected_sha = sha256_hex(&glb);
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let model_url = format!("{}/model.glb?sign=budget", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) = model_app("t13-budget", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let settings = app.state().settings().clone();
    let downloader = ModelDownloader::new(
        app.dir(),
        DownloadPolicy {
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            allow_local_fixture: true,
            ..DownloadPolicy::from_settings(&settings)
        },
    );
    let executor = injected_executor(
        &app,
        Arc::clone(&clock),
        downloader,
        GlbBudget {
            max_triangles: 4,
            max_texture_dimension: 4096,
        },
    );
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        80,
    )
    .await;

    let stage = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    let needs_input = stage.needs_input_json.clone().unwrap();
    assert!(
        needs_input.to_string().contains("gltf_face_limit"),
        "{needs_input}"
    );
    assert!(
        needs_input.to_string().contains("原始模型已保留"),
        "{needs_input}"
    );
    // 原始模型保留：blob 文件、asset、rejected revision 都在。
    let blob_path = app
        .dir()
        .join("blobs")
        .join(&expected_sha[..2])
        .join(&expected_sha);
    assert!(blob_path.is_file(), "原始模型必须保留");
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &item_id, &expected_sha)
        .await
        .unwrap()
        .expect("rejected revision");
    assert_eq!(revision.validation_state, ModelValidationState::Rejected);
    assert!(revision.bounds.is_none(), "未通过校验不得写 bounds");
    assert_eq!(stage.usage_json.unwrap()["validation"], "rejected");
    // 不重新购买、不重新下载。
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
    cdn.assert_called_times("GET", "/model.glb", 1);
}

/// 截断 GLB（下载成功但文件本身被截断）→ `needs_input` 且原始模型保留。
#[tokio::test]
async fn truncated_glb_enters_needs_input_and_keeps_the_original() {
    let mut glb = build_glb();
    glb.truncate(glb.len() / 2);
    let dir = TestDir::new("t13-truncated");
    let truncated_path = write_temp_model(dir.path(), "truncated.glb", &glb);
    let truncated_sha = sha256_hex(&glb);

    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file(&truncated_path)],
    )]));
    let model_url = format!("{}/model.glb?sign=truncated", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-truncated", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = model_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        80,
    )
    .await;

    let stage = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    let needs_input = stage.needs_input_json.clone().unwrap().to_string();
    assert!(needs_input.contains("glb_declared_length"), "{needs_input}");
    assert!(needs_input.contains("原始模型已保留"), "{needs_input}");
    // 原始（截断的）模型仍以资产形式保留，便于人工核对。
    let blob_path = app
        .dir()
        .join("blobs")
        .join(&truncated_sha[..2])
        .join(&truncated_sha);
    assert!(blob_path.is_file(), "原始模型必须保留");
    let mut conn = pool.acquire().await.unwrap();
    let revision = revisions_repo::find_by_sha(&mut conn, &item_id, &truncated_sha)
        .await
        .unwrap()
        .expect("rejected revision");
    assert_eq!(revision.validation_state, ModelValidationState::Rejected);
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
}

/// 磁盘满（注入空间探测）→ `needs_input`；不产生模型资产、不半提交。
#[tokio::test]
async fn disk_full_enters_needs_input_without_partial_artifact() {
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let model_url = format!("{}/model.glb?sign=disk", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-disk-full", &format!("{}/v3", server.base_url())).await;
    let (job_id, item_id) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let settings = app.state().settings().clone();
    let downloader = ModelDownloader::new(
        app.dir(),
        DownloadPolicy {
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            allow_local_fixture: true,
            ..DownloadPolicy::from_settings(&settings)
        },
    )
    .with_space_probe(SpaceProbe::Fixed(0));
    let executor = injected_executor(&app, Arc::clone(&clock), downloader, GlbBudget::default());
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::NeedsInput,
        80,
    )
    .await;

    let stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    let needs_input = stage.needs_input_json.clone().unwrap().to_string();
    assert!(
        needs_input.contains("download_insufficient_storage"),
        "{needs_input}"
    );
    assert!(stage.result_asset_id.is_none(), "不得留下半提交结果");
    // 没有任何 purpose=model 的资产；tmp 无残留。
    let mut conn = pool.acquire().await.unwrap();
    let model_assets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE item_id = ? AND purpose = 'model'")
            .bind(&item_id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(model_assets, 0, "磁盘满不得产生模型资产");
    assert_no_tmp_leftovers(app.dir());
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
}

/// 生产策略（无测试放行）在真实执行器上仍拒绝本机 fixture 的模型链接（
/// 证明"测试放行"必须显式配置；缺配置时是 needs_input 而不是静默成功）。
#[tokio::test]
async fn unconfigured_allowlist_blocks_download_in_needs_input() {
    let cdn = FixtureServer::start(scenario(vec![exact_route(
        "GET",
        "/model.glb",
        vec![respond_file("assets/sample-model.glb")],
    )]));
    let model_url = format!("{}/model.glb?sign=x", cdn.base_url());
    let server = FixtureServer::start(scenario(vec![
        exact_route("POST", "/v3/files", upload_steps()),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_json(
                json!({ "code": 0, "data": { "task_id": TASK_ID } }),
            )],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_json(task_success_with(&model_url, json!(null)))],
        ),
    ]));
    let (app, cookie, csrf) =
        model_app("t13-no-allowlist", &format!("{}/v3", server.base_url())).await;
    // 清空允许域名单（等价"未配置允许域"）：拒绝下载并给出可行动缺项。
    let mut settings = app.state().settings().clone();
    settings.download.allowed_hosts.clear();
    let (job_id, _) = create_ready_job(&app, &cookie, &csrf).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let mut registry = StageRegistry::new();
    let handlers = TripoHandlers::from_settings(&settings).expect("构造");
    handlers.register(&mut registry);
    let clock_dyn: Arc<dyn everything_manual::jobs::Clock> = clock.clone();
    let executor =
        fixed_jitter_executor(pool(&app), ExecutorConfig::default(), registry, clock_dyn);
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelDownload,
        JobStatus::NeedsInput,
        80,
    )
    .await;
    let stage = stage_of(&pool, &job_id, StageKind::ModelDownload).await;
    let needs_input = stage.needs_input_json.clone().unwrap().to_string();
    assert!(
        needs_input.contains("download_host_not_allowed"),
        "{needs_input}"
    );
    assert_eq!(
        cdn.call_count("GET", "/model.glb"),
        0,
        "被拒的目标不得被连接"
    );
    server.assert_called_times("POST", "/v3/generation/multiview-to-model", 1);
}
