//! 仅测试依赖的 fixture 支撑库（REQ-008 / AC-014；T05 交付）。
//!
//! 本 crate **只作为 dev-dependency** 使用，不进入任何生产依赖树：
//! `crates/server/Cargo.toml` 只把它列在 `[dev-dependencies]`，
//! 由 `crates/server/tests/fixture_harness.rs` 的清单断言与 `cargo tree` 证据守护。
//!
//! 组成：
//! - [`server::FixtureServer`]：只绑定 `127.0.0.1:0` 的脚本化 HTTP fixture，
//!   支持成功/延迟/断连/RST/半关闭截断/429/5xx/畸形 JSON/超时的字节级行为，
//!   记录每次调用的方法/路径/请求头（脱敏）/请求体/次数，缺脚本必须失败；
//! - [`scenario`]：场景 JSON（`tests/fixtures/scenarios/*.json`）与字节级解析；
//! - [`client::LocalHttpClient`]：仅回环的测试 HTTP 客户端（拒绝 DNS 与公网地址，
//!   保证测试进程无法发起真实外网调用）；
//! - [`assets`]：原创样例资产的**结构级**校验（GLB/PDF/PNG/JPEG）与 sha256；
//! - [`presets`]：Tripo / 说明书 AI 的路径常量与预置场景入口（T12/T14 复用）。
//!
//! 生成原始样例资产的工具：`cargo run -p test-support --bin generate-fixtures`
//! （见 `tests/fixtures/README.md` 的来源与许可记录）。

pub mod assets;
pub mod client;
pub mod generate;
pub mod presets;
pub mod record;
pub mod scenario;
pub mod server;

pub use assets::{
    GlbInfo, JpegInfo, PdfInfo, PngInfo, sha256_hex, validate_glb, validate_jpeg, validate_pdf,
    validate_png,
};
pub use client::{ClientError, HttpResponse, LocalHttpClient};
pub use record::{HeaderView, RecordedRequest, RecordingOutcome};
pub use scenario::{
    BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step, fixtures_root,
};
pub use server::{FixtureServer, ScriptProblem, ScriptProblemKind};
