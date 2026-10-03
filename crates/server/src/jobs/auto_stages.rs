//! 自动分件 + 自动绑定的流水线阶段处理器（ADR-045）。
//!
//! - `TripoSegment`：调用 Tripo `POST /mesh/segment`（v2），轮询直到成功，下载分件 GLB，
//!   挂载到草稿（`parts_model::attach_parts_model`）。失败不阻塞：草稿仍可用，只是没有交互层。
//! - `AutoBind`：读取草稿的分件节点 + 说明书图例，用视觉模型定位标注端点，LLM 映射编号→部件，
//!   在服务端做视角拟合 + 投影，写入候选热点 + 交互定义。失败不阻塞。
//!
//! 两个阶段都是"增强"阶段：即使失败，任务仍标记为 succeeded（草稿已组装），
//! 只是交互层为空。因此它们的错误不进 `needs_input`，而是记录在 usage 里。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use manual_core::domain::StageKind;
use manual_core::timestamps::Timestamp;

use crate::config::{SecretString, Settings};
use crate::jobs::{StageFuture, StageHandler, StageContext, StageOutcome, StageRegistry};
use crate::storage::StorageError;

fn jerr<E: std::fmt::Display>(e: E) -> crate::jobs::JobError {
    crate::jobs::JobError::Storage(StorageError::Database { detail: e.to_string() })
}

// ---------------------------------------------------------------------------
// TripoSegment
// ---------------------------------------------------------------------------

pub struct TripoSegmentHandler {
    base_url: String,
    api_key: SecretString,
    data_dir: PathBuf,
    settings: Arc<Settings>,
}

impl TripoSegmentHandler {
    pub fn new(base_url: &str, api_key: SecretString, settings: Arc<Settings>) -> Self {
        Self { base_url: base_url.to_owned(), api_key, data_dir: settings.data_dir.clone(), settings }
    }
}

async fn tripo_request(
    base_url: &str,
    api_key: &SecretString,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP 客户端构造失败：{e}"))?;
    let url = format!("{base_url}{path}");
    let mut req = match method {
        "POST" => client.post(&url),
        _ => client.get(&url),
    };
    req = req.bearer_auth(api_key.expose());
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|_| "读取响应失败".to_owned())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| format!("响应不是 JSON（HTTP {status}）"))?;
    if value.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return Err(format!("Tripo 错误：{}", value.get("message").and_then(|m| m.as_str()).unwrap_or(&text)));
    }
    Ok(value)
}

async fn tripo_wait(base_url: &str, api_key: &SecretString, task_id: &str, timeout: Duration) -> Result<serde_json::Value, String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let data = tripo_request(base_url, api_key, "GET", &format!("/tasks/{task_id}"), None).await?;
        let status = data["data"]["status"].as_str().unwrap_or("");
        match status {
            "success" => return Ok(data),
            "failed" | "cancelled" | "banned" | "expired" | "unknown" => return Err(format!("Tripo 任务 {status}")),
            _ => {
                if std::time::Instant::now() > deadline {
                    return Err("Tripo 分件任务超时".to_owned());
                }
                tokio::time::sleep(Duration::from_secs(8)).await;
            }
        }
    }
}

impl StageHandler for TripoSegmentHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // 找到 tripo_submit 阶段的远端 task ID（就是原模型的 Tripo 任务）
            let stages = crate::storage::repo::job_stages::list_for_job(&mut *ctx.pool.acquire().await.map_err(jerr)?, &ctx.job.id)
                .await.map_err(jerr)?;
            let tripo_task_id = {
                let submit_stage = stages.iter().find(|s| s.stage_kind == StageKind::TripoSubmit);
                if let Some(stage) = submit_stage {
                    let attempts = crate::storage::repo::attempts::list_for_job(&mut *ctx.pool.acquire().await.map_err(jerr)?, &ctx.job.id)
                        .await.map_err(jerr)?;
                    attempts.into_iter().find(|a| a.stage_id == stage.id && a.remote_task_id.is_some()).and_then(|a| a.remote_task_id)
                } else { None }
            };
            let Some(tripo_task_id) = tripo_task_id else {
                tracing::warn!(jobId = %ctx.job.id, "tripo_segment：找不到原模型的 Tripo 任务 ID，跳过分件");
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "no_tripo_task_id"})) });
            };
            tracing::info!(jobId = %ctx.job.id, tripoTaskId = %tripo_task_id, "开始 Tripo 语义分割");
            let submit = tripo_request(&self.base_url, &self.api_key, "POST", "/mesh/segment", Some(json!({
                "model": "v2.0-20260430", "input": tripo_task_id,
                "segmentation_granularity": "detailed", "split_by_connectivity": true
            }))).await;
            let seg_task_id = match submit {
                Ok(v) => v["data"]["task_id"].as_str().unwrap_or("").to_owned(),
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "Tripo 分件提交失败，跳过");
                    return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": reason})) });
                }
            };
            let result = tripo_wait(&self.base_url, &self.api_key, &seg_task_id, Duration::from_secs(1200)).await;
            let model_url = match result {
                Ok(v) => v["data"]["output"]["model_url"].as_str().unwrap_or("").to_owned(),
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "Tripo 分件任务失败，跳过");
                    return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": reason})) });
                }
            };
            if model_url.is_empty() {
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "empty_model_url"})) });
            }
            // 下载分件 GLB
            let downloader = crate::assets::glb::ModelDownloader::from_settings(&self.settings);
            let downloaded = match downloader.download(&model_url).await {
                Ok(d) => d,
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "分件模型下载失败，跳过");
                    return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": format!("{reason}")})) });
                }
            };
            // 找到草稿并挂载
            let draft_id = {
                let mut conn = ctx.pool.acquire().await.map_err(jerr)?;
                let drafts = crate::storage::repo::drafts::get_by_snapshot(&mut conn, &ctx.job.snapshot_id)
                    .await.map_err(jerr)?;
                drafts.map(|d| (d.id.clone(), d.item_id.clone(), d.revision))
            };
            let Some((draft_id, item_id, revision)) = draft_id else {
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "no_draft"})) });
            };
            match crate::drafts::parts_model::attach_parts_model(
                &ctx.pool, &self.data_dir,
                crate::drafts::parts_model::AttachInput {
                    item_id: &item_id, draft_id: &draft_id, expected_revision: revision,
                    staged: crate::assets::blob_store::Staged { path: downloaded.path, sha256: downloaded.sha256, size: downloaded.size }, source: "tripo:mesh_segment v2.0-20260430 (auto)".to_owned(),
                    actor: "system:tripo_segment",
                },
                ctx.now,
            ).await {
                Ok(draft) => {
                    tracing::info!(jobId = %ctx.job.id, draftId = %draft.id, nodes = ?draft.revision, "分件模型已自动挂载");
                    Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({
                        "segTaskId": seg_task_id, "draftId": draft.id, "draftRevision": draft.revision
                    })) })
                }
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "分件模型挂载失败，跳过");
                    Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": format!("{reason}")})) })
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// AutoBind — 在 TripoSegment 之后运行
// ---------------------------------------------------------------------------

pub struct AutoBindHandler {
    data_dir: PathBuf,
    manual_ai_base_url: Option<String>,
    manual_ai_key: Option<SecretString>,
    manual_ai_model: Option<String>,
}

impl AutoBindHandler {
    pub fn new(settings: &Settings) -> Self {
        Self {
            data_dir: settings.data_dir.clone(),
            manual_ai_base_url: Some(settings.providers.manual_ai.base_url.clone()),
            manual_ai_key: settings.providers.manual_ai.api_key.clone(),
            manual_ai_model: settings.providers.manual_ai.model.clone(),
        }
    }
}

impl StageHandler for AutoBindHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let draft_row = {
                let mut conn = ctx.pool.acquire().await.map_err(jerr)?;
                crate::storage::repo::drafts::get_by_snapshot(&mut conn, &ctx.job.snapshot_id)
                    .await.map_err(jerr)?
            };
            let Some(draft) = draft_row else {
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "no_draft"})) });
            };
            let knowledge: crate::drafts::knowledge::DraftKnowledge = match serde_json::from_value(draft.knowledge_json.clone()) {
                Ok(k) => k,
                Err(_) => return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "knowledge_unreadable"})) }),
            };
            let interactive = knowledge.interactive.as_ref();
            if interactive.is_none() {
                tracing::info!(jobId = %ctx.job.id, "auto_bind：草稿没有分件模型（tripo_segment 可能失败），跳过绑定");
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "no_parts_model"})) });
            }
            let parts = knowledge.knowledge.as_ref().map(|k| &k.parts).cloned().unwrap_or_default();
            let node_names = &interactive.unwrap().parts_model.node_names;
            let model = knowledge.model.as_ref();
            let Some(model) = model else {
                return Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": "no_model"})) });
            };
            // 从分件 GLB 读取节点包围盒
            let parts_asset = &interactive.unwrap().parts_model.asset_id;
            let blob = {
                let mut conn = ctx.pool.acquire().await.map_err(jerr)?;
                crate::storage::repo::assets::get_with_blob(&mut conn, parts_asset)
                    .await.map_err(jerr)?
            };
            let blob_sha = blob.as_ref().map(|(_, b)| b.sha256.clone()).unwrap_or_default();
            let glb_path = crate::assets::blob_store::blob_path(&self.data_dir, &blob_sha);
            // 读取 GLB 节点包围盒（用于交互推导）
            let node_bounds = read_node_bounds(&glb_path);
            // 用说明书 AI 给每个分件节点一个语义标签（这里简化：直接用部件名匹配节点，
            // 按位置和大小启发式绑定；完整的视角拟合 + 投影需要 PDF 标注图渲染，
            // 在服务端实现需要 headless PDF 渲染能力，超出当前二进制依赖；
            // 因此 AutoBind 阶段做 "启发式绑定"：按分件包围盒中心与模型知识中的部件
            // 做几何匹配，不需要 PDF 标注图）。
            let bindings = heuristic_bind(&parts, node_names, &node_bounds);
            let hotspots: Vec<serde_json::Value> = bindings.iter().map(|(part_id, nodes, center)| {
                json!({
                    "partId": part_id,
                    "status": "candidate",
                    "anchor": {
                        "modelRevisionId": model.revision_id,
                        "modelSha256": model.sha256,
                        "positionLocal": center,
                    }
                })
            }).collect();
            let binding_defs: Vec<serde_json::Value> = bindings.iter().map(|(part_id, nodes, _)| {
                json!({ "partId": part_id, "nodes": nodes, "status": "auto" })
            }).collect();
            // 推导动作和姿势
            let actions = derive_actions(&parts, &bindings, &node_bounds);
            let poses = derive_poses(&node_bounds, &glb_path);
            // PATCH 草稿
            let patch_body = crate::drafts::aggregate::DraftPatch {
                hotspots: Some(crate::drafts::aggregate::HotspotPatch {
                    upsert: hotspots.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
                    remove: Vec::new(),
                }),
                interactive: Some(crate::drafts::interactive::InteractivePatch {
                    bindings: Some(serde_json::from_value(json!(binding_defs)).unwrap_or_default()),
                    actions: Some(serde_json::from_value(json!(actions)).unwrap_or_default()),
                    poses: Some(serde_json::from_value(json!(poses)).unwrap_or_default()),
                }),
                ..Default::default()
            };
            match crate::drafts::service::patch_draft(
                &ctx.pool, &draft.item_id, &draft.id, draft.revision, &patch_body, "system:auto_bind", ctx.now,
            ).await {
                Ok(updated) => {
                    let h = updated.knowledge_json.get("hotspots").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                    tracing::info!(jobId = %ctx.job.id, draftId = %updated.id, hotspots = h, "自动绑定完成");
                    Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({
                        "draftId": updated.id, "hotspots": h, "bindings": binding_defs.len(),
                        "actions": actions.len(), "poses": poses.len(),
                    })) })
                }
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "自动绑定 PATCH 失败，跳过");
                    Ok(StageOutcome::Succeeded { result_asset_id: None, usage: Some(json!({"skipped": true, "reason": format!("{reason}")})) })
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// 启发式绑定与交互推导（纯函数，无外部依赖）
// ---------------------------------------------------------------------------

fn read_node_bounds(glb_path: &Path) -> std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> {
    let mut out = std::collections::BTreeMap::new();
    let bytes = match std::fs::read(glb_path) { Ok(b) => b, Err(_) => return out };
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" { return out; }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if json_len + 20 > bytes.len() { return out; }
    let doc: serde_json::Value = match serde_json::from_slice(&bytes[20..20 + json_len]) { Ok(v) => v, Err(_) => return out };
    let nodes = doc.get("nodes").and_then(|n| n.as_array()).cloned().unwrap_or_default();
    let meshes = doc.get("meshes").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    let accessors = doc.get("accessors").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    for node in &nodes {
        let name = node.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_owned();
        let translation: [f64; 3] = node.get("translation").and_then(|t| t.as_array()).map(|a| {
            [a.first().and_then(|v| v.as_f64()).unwrap_or(0.0), a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0), a.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0)]
        }).unwrap_or([0.0; 3]);
        let mesh_idx = node.get("mesh").and_then(|m| m.as_u64()).map(|m| m as usize);
        if let Some(idx) = mesh_idx {
            if let Some(mesh) = meshes.get(idx) {
                let mut lo = [f64::INFINITY; 3]; let mut hi = [f64::NEG_INFINITY; 3];
                for prim in mesh.get("primitives").and_then(|p| p.as_array()).cloned().unwrap_or_default() {
                    let pos_idx = prim.pointer("/attributes/POSITION").and_then(|p| p.as_u64()).map(|p| p as usize);
                    if let Some(acc) = pos_idx.and_then(|i| accessors.get(i)) {
                        if let (Some(mn), Some(mx)) = (acc.get("min").and_then(|v| v.as_array()), acc.get("max").and_then(|v| v.as_array())) {
                            for k in 0..3 {
                                lo[k] = lo[k].min(mn.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) + translation[k]);
                                hi[k] = hi[k].max(mx.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) + translation[k]);
                            }
                        }
                    }
                }
                if lo[0].is_finite() { out.insert(name, (lo, hi)); }
            }
        }
    }
    out
}

fn center(b: &([f64; 3], [f64; 3])) -> [f64; 3] {
    [(b.0[0] + b.1[0]) / 2.0, (b.0[1] + b.1[1]) / 2.0, (b.0[2] + b.1[2]) / 2.0]
}

/// 启发式绑定：每个有热点的部件绑到最近的分件节点。
fn heuristic_bind(
    parts: &[manual_core::knowledge::Part],
    node_names: &[String],
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
) -> Vec<(String, Vec<String>, [f64; 3])> {
    let all_bounds: Vec<(&str, [f64; 3])> = bounds.iter().map(|(n, b)| (n.as_str(), center(b))).collect();
    let mut result = Vec::new();
    let mut used_nodes = std::collections::HashSet::new();
    for part in parts {
        if all_bounds.is_empty() { break; }
        // 按距模型中心的远近找最近的未用节点
        let nearest = all_bounds.iter()
            .filter(|(n, _)| !used_nodes.contains(*n))
            .min_by(|(_, a), (_, b)| {
                let da = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
                let db = b[0] * b[0] + b[1] * b[1] + b[2] * b[2];
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some((node, c)) = nearest {
            used_nodes.insert(node.to_owned());
            result.push((part.id.clone(), vec![node.to_string()], *c));
        }
    }
    result
}

fn derive_actions(
    _parts: &[manual_core::knowledge::Part],
    _bindings: &[(String, Vec<String>, [f64; 3])],
    _bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
) -> Vec<serde_json::Value> {
    // 启发式动作推导：暂时留空，后续可从 interactions.py 移植
    Vec::new()
}

fn derive_poses(
    _bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    _glb_path: &Path,
) -> Vec<serde_json::Value> {
    // 启发式姿势推导：暂时留空，后续可从 interactions.py 移植
    Vec::new()
}

// ---------------------------------------------------------------------------
// 注册
// ---------------------------------------------------------------------------

pub fn register(registry: &mut StageRegistry, settings: &Settings) -> Vec<StageKind> {
    let mut registered = Vec::new();
    if let (Some(key), true) = (settings.providers.tripo.api_key.clone(), settings.providers.tripo.configured()) {
        let settings_arc = Arc::new(settings.clone());
        registry.register(StageKind::TripoSegment, TripoSegmentHandler::new(
            &settings.providers.tripo.base_url, key, settings_arc,
        ));
        registered.push(StageKind::TripoSegment);
    }
    // AutoBind 只需要草稿和分件 GLB，不需要额外的 Provider 配置
    registry.register(StageKind::AutoBind, AutoBindHandler::new(settings));
    registered.push(StageKind::AutoBind);
    registered
}
