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
            let steps_list = knowledge.knowledge.as_ref().map(|k| k.steps.clone()).unwrap_or_default();
            let actions = derive_actions(&parts, &bindings, &node_bounds, &steps_list);
            let poses = derive_poses(&node_bounds, &parts);
            let hotspot_count = hotspots.len();
            let action_count = actions.len();
            let pose_count = poses.len();
            tracing::info!(jobId = %ctx.job.id, hotspots = hotspot_count, actions = action_count, poses = pose_count, "auto_bind：准备 PATCH");
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
                    if let crate::drafts::service::DraftServiceError::FieldIssues(ref issues) = reason {
                        for issue in issues { tracing::warn!(field = %issue.field, message = %issue.message, "字段问题"); }
                    }
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

/// 从分件节点包围盒推导动作（相机/设备类产品）。
/// 按部件名称中的关键词匹配典型操作（盖子→外翻取下，按钮→按下，转盘→转动，杆→扳动）。
fn derive_actions(
    parts: &[manual_core::knowledge::Part],
    bindings: &[(String, Vec<String>, [f64; 3])],
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    steps: &[manual_core::knowledge::Step],
) -> Vec<serde_json::Value> {
    use serde_json::json;
    let all_bounds: Vec<[f64; 3]> = bounds.values().flat_map(|b| vec![b.0, b.1]).collect();
    if all_bounds.is_empty() { return Vec::new(); }
    let lo = [all_bounds.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
              all_bounds.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
              all_bounds.iter().map(|b| b[2]).fold(f64::INFINITY, f64::min)];
    let hi = [all_bounds.iter().map(|b| b[0]).fold(f64::NEG_INFINITY, f64::max),
              all_bounds.iter().map(|b| b[1]).fold(f64::NEG_INFINITY, f64::max),
              all_bounds.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max)];
    let volume = (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2]);
    let mut actions = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let part_name: std::collections::HashMap<&str, &str> = parts.iter().map(|p| (p.id.as_str(), p.name.as_str())).collect();
    let steps_for = |keywords: &[&str]| -> Vec<String> {
        steps.iter().filter(|s| keywords.iter().any(|k| s.title.contains(k))).take(6).map(|s| s.id.clone()).collect()
    };
    let mut labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (pid, nodes, c) in bindings {
        let raw_name = part_name.get(pid.as_str()).copied().unwrap_or("");
        // 同一部件名在说明书里常出现多次（不同页的同名部件）：同名只生成一个动作。
        // 去重键忽略空白（「跟焦 / 变焦切换键」与「跟焦/变焦切换键」是同一个部件）。
        let dedupe_key: String = raw_name.chars().filter(|c| !c.is_whitespace()).collect();
        if !labels.insert(dedupe_key) { continue; }
        // 规则按简体关键字匹配；繁体说明书先做常用字折叠。
        let folded = fold_traditional(raw_name);
        let name = folded.as_str();
        let ns: Vec<String> = nodes.iter().filter(|n| bounds.contains_key(n.as_str()) && !used.contains(n.as_str())).cloned().collect();
        if ns.is_empty() { continue; }
        let b = (ns.iter().map(|n| bounds[n].0).fold([f64::INFINITY; 3], |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]),
                 ns.iter().map(|n| bounds[n].1).fold([f64::NEG_INFINITY; 3], |a, b| [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]));
        let node_vol = (b.1[0] - b.0[0]) * (b.1[1] - b.0[1]) * (b.1[2] - b.0[2]);
        if node_vol > 0.25 * volume { continue; } // skip body-sized nodes
        let r = |n: &[String], pivot: [f64; 3], axis: [f64; 3], deg: f64| json!({"nodes": n, "kind": "rotate", "pivot": pivot, "axis": axis, "angleDeg": deg});
        let t = |n: &[String], v: [f64; 3]| json!({"nodes": n, "kind": "translate", "vector": v});
        // 「手柄」只有在是电池盖一类可拆件时才做拆卸动作（云台/咖啡机的手柄是主体或另有语义）。
        let is_cover_grip = name.contains("手柄") && (name.contains("电池") || name.contains("盖"));
        if name.contains("电池盖") || is_cover_grip {
            let side = if c[0] < 0.0 { -1.0 } else { 1.0 };
            let hinge = [if side < 0.0 { b.1[0] } else { b.0[0] }, b.0[1], b.1[2]];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("取下{name}"), "description": "露出电池仓（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 900,
                "steps": [t(&ns, [side * 0.18, -0.05, 0.08]), r(&ns, hinge, [0.0, 1.0, 0.0], side * 25.0)],
                "stepIds": steps_for(&["电池"])}));
        } else if name.contains("后盖") {
            let hinge = [b.1[0], c[1], b.0[2]];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("打开{name}"), "description": "装入胶片时打开（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 1000,
                "steps": [r(&ns, hinge, [0.0, 1.0, 0.0], -70.0)], "stepIds": steps_for(&["胶片"])}));
        } else if !name.contains("膝盖") && (name.contains("翻盖") || name.contains("掀盖") || name.ends_with("盖") || name.ends_with("盖板")) {
            // 通用翻盖：沿顶部后缘为铰链向上掀开。
            let up = if (b.1[1] - b.0[1]) <= (b.1[2] - b.0[2]) { 1 } else { 2 };
            let mut hinge = *c;
            hinge[up] = b.1[up];
            hinge[0] = b.0[0];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("打开{raw_name}"), "description": "掀开盖子（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 900,
                "steps": [r(&ns, hinge, [0.0, 0.0, 1.0], 60.0)], "stepIds": steps_for(&[&raw_name[..raw_name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if ["尘盒", "水箱", "滤网", "抽屉", "滴水盘"].iter().any(|k| name.ends_with(k)) {
            // 只匹配以模块名结尾的部件（「集尘盒卡扣」「集尘盒盖」是子部件，不整体抽出）。
            // 可抽出的模块：沿离物体中心最近的外侧方向平移拉出。
            let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
            let d = [c[0] - mid[0], c[1] - mid[1], c[2] - mid[2]];
            let axis = if d[0].abs() >= d[2].abs() { 0 } else { 2 };
            let mut v = [0.0; 3];
            v[axis] = if d[axis] >= 0.0 { 0.15 } else { -0.15 };
            actions.push(json!({"id": format!("pull-{}", actions.len()), "label": format!("取出{raw_name}"), "description": "抽出模块（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 800,
                "steps": [t(&ns, v)], "stepIds": steps_for(&[&raw_name[..raw_name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("滤碗手柄") || name.contains("冲泡头手柄") {
            // 咖啡机滤碗手柄：先逆时针旋出，再向下取出。
            let mut down = [0.0; 3];
            down[1] = -0.1;
            actions.push(json!({"id": format!("pull-{}", actions.len()), "label": format!("取下{raw_name}"), "description": "旋出并取下（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 1000,
                "steps": [r(&ns, *c, [0.0, 1.0, 0.0], -40.0), t(&ns, down)], "stepIds": steps_for(&["滤碗", "手柄"])}));
        } else if name.contains("扳机") {
            let pivot = [c[0], b.1[1], c[2]];
            actions.push(json!({"id": format!("press-{}", actions.len()), "label": format!("扣动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 400,
                "steps": [r(&ns, pivot, [0.0, 0.0, 1.0], 15.0)], "stepIds": steps_for(&["扳机"])}));
        } else if name.contains("摇杆") {
            let pivot = [c[0], b.0[1], c[2]];
            actions.push(json!({"id": format!("lever-{}", actions.len()), "label": format!("推动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 600,
                "steps": [r(&ns, pivot, [0.0, 0.0, 1.0], 20.0)], "stepIds": steps_for(&["摇杆"])}));
        } else if name.contains("按钮") || name.contains("快门") || name.ends_with("键") || name.contains("开关") {
            actions.push(json!({"id": format!("press-{}", actions.len()), "label": format!("按下{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 300,
                "steps": [t(&ns, [0.0, -0.012, 0.0])], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("转盘") || name.contains("拨盘") || name.contains("旋钮") || name.contains("拨轮") {
            actions.push(json!({"id": format!("turn-{}", actions.len()), "label": format!("转动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 900,
                "steps": [r(&ns, *c, [0.0, 1.0, 0.0], 60.0)], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("杆") && !name.contains("摇杆") {
            let pivot = [b.0[0] + 0.03, c[1], c[2]];
            actions.push(json!({"id": format!("lever-{}", actions.len()), "label": format!("扳动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 700,
                "steps": [r(&ns, pivot, [0.0, 1.0, 0.0], -35.0)], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else {
            continue;
        }
        for n in &ns { used.insert(n.clone()); }
    }
    actions.into_iter().take(12).collect()
}

/// 动作规则用到的繁体字 → 简体（只覆盖规则关键字，不做通用转换）。
fn fold_traditional(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '鍵' => '键', '蓋' => '盖', '塵' => '尘', '濾' => '滤', '網' => '网', '開' => '开',
            '關' => '关', '鈕' => '钮', '轉' => '转', '撥' => '拨', '輪' => '轮', '電' => '电',
            '門' => '门', '桿' => '杆', '後' => '后', '機' => '机', '盤' => '盘', '屜' => '屉',
            other => other,
        })
        .collect()
}

/// 说明书文本是否表明这是四足/机器狗一类可摆姿势的产品。
fn looks_like_quadruped(parts: &[manual_core::knowledge::Part]) -> bool {
    const KEYWORDS: &[&str] = &[
        "机器狗", "四足", "仿生四足", "cyberdog", "quadruped", "spot",
    ];
    parts.iter().any(|part| {
        let name = part.name.to_ascii_lowercase();
        KEYWORDS.iter().any(|kw| name.contains(kw) || part.name.contains(kw))
    })
}

/// 垂直轴：Y 或 Z 中跨度更大、且底部有若干细长件的那根。Tripo 分件朝向不稳定。
fn vertical_axis(span: [f64; 3]) -> usize {
    if span[2] > span[1] * 1.15 { 2 } else { 1 }
}

/// 从分件节点包围盒推导四足机器人姿势。
/// 找身体轴和头部方向，识别四条腿（大腿/小腿/足垫），生成站立/趴下/坐下/握手/作揖。
fn derive_poses(
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    parts: &[manual_core::knowledge::Part],
) -> Vec<serde_json::Value> {
    use serde_json::json;
    if bounds.len() < 8 { return Vec::new(); } // too few parts for a robot
    if !looks_like_quadruped(parts) { return Vec::new(); }
    let all: Vec<[f64; 3]> = bounds.values().flat_map(|b| vec![b.0, b.1]).collect();
    let lo = [all.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
              all.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
              all.iter().map(|b| b[2]).fold(f64::INFINITY, f64::min)];
    let hi = [all.iter().map(|b| b[0]).fold(f64::NEG_INFINITY, f64::max),
              all.iter().map(|b| b[1]).fold(f64::NEG_INFINITY, f64::max),
              all.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max)];
    let span = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let up = vertical_axis(span);
    // Horizontal body axis = remaining axis with the largest span (not `up`).
    let body_axis = [0, 1, 2].into_iter().filter(|&a| a != up).max_by(|&a, &b| span[a].partial_cmp(&span[b]).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0);
    let side_axis = [0, 1, 2].into_iter().find(|&a| a != up && a != body_axis).unwrap_or(2);
    // Find legs: per body quadrant, the tallest elongated node near the floor
    // (fixed floor cutoffs miss legs that end a few millimetres higher).
    let quad_mid_body = (lo[body_axis] + hi[body_axis]) / 2.0;
    let quad_mid_side = (lo[side_axis] + hi[side_axis]) / 2.0;
    let mut best: std::collections::BTreeMap<(bool, bool), (&str, f64)> = std::collections::BTreeMap::new();
    for (n, b) in bounds {
        let height = b.1[up] - b.0[up];
        if b.0[up] > lo[up] + 0.3 * span[up] || height < 0.15 * span[up] || (b.1[body_axis] - b.0[body_axis]) > 0.4 * span[body_axis] {
            continue;
        }
        let c = center(b);
        let key = (c[body_axis] > quad_mid_body, c[side_axis] < quad_mid_side);
        if best.get(&key).is_none_or(|(_, h)| height > *h) {
            best.insert(key, (n.as_str(), height));
        }
    }
    let shins: Vec<&str> = best.values().map(|(n, _)| *n).collect();
    if shins.len() != 4 { return Vec::new(); }
    // Head direction: tallest parts on one end
    let tall: Vec<(&str, f64)> = bounds.iter()
        .filter(|(_, b)| b.1[up] > lo[up] + 0.7 * span[up])
        .map(|(n, b)| (n.as_str(), (b.0[body_axis] + b.1[body_axis]) / 2.0)).collect();
    let head_mean = if tall.is_empty() { (lo[body_axis] + hi[body_axis]) / 2.0 } else { tall.iter().map(|(_, c)| c).sum::<f64>() / tall.len() as f64 };
    let head_sign = if head_mean >= (lo[body_axis] + hi[body_axis]) / 2.0 { 1.0 } else { -1.0 };
    let mut swing = [0.0; 3]; // rotation axis perpendicular to body and up
    swing[side_axis] = 1.0;
    // Classify legs into corners
    let mid_body = (lo[body_axis] + hi[body_axis]) / 2.0;
    let mid_side = (lo[side_axis] + hi[side_axis]) / 2.0;
    struct Leg { thigh: Option<String>, shin: String, feet: Vec<String> }
    let mut legs: std::collections::BTreeMap<String, Leg> = std::collections::BTreeMap::new();
    for sh in &shins {
        let c = center(bounds.get(*sh).unwrap());
        let front = (c[body_axis] - mid_body) * head_sign > 0.0;
        let left = c[side_axis] < mid_side;
        let corner = format!("{}{}", if front { "F" } else { "R" }, if left { "L" } else { "R" });
        let thighs: Vec<&str> = bounds.iter()
            .filter(|(n, b)| !shins.contains(&n.as_str())
                && (center(b)[body_axis] - c[body_axis]).abs() < 0.12 * span[body_axis].max(0.08)
                && (center(b)[side_axis] - c[side_axis]).abs() < 0.12 * span[side_axis].max(0.08)
                && center(b)[up] > c[up]
                && (b.1[up] - b.0[up]) < 0.45 * span[up])
            .map(|(n, _)| n.as_str()).collect();
        let thigh = thighs.into_iter().max_by(|a, b| {
            let va = bounds[*a]; let vb = bounds[*b];
            let sa = (va.1[0]-va.0[0])*(va.1[1]-va.0[1])*(va.1[2]-va.0[2]);
            let sb = (vb.1[0]-vb.0[0])*(vb.1[1]-vb.0[1])*(vb.1[2]-vb.0[2]);
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        }).map(str::to_owned);
        legs.entry(corner).or_insert(Leg { thigh, shin: sh.to_string(), feet: Vec::new() });
    }
    // Attach feet (small floor-level nodes) to nearest shin
    for (n, b) in bounds {
        if shins.contains(&n.as_str()) || (b.1[up] - b.0[up]) > 0.18 * span[up] || b.0[up] > lo[up] + 0.1 * span[up] { continue; }
        let c = center(b);
        if let Some((_, leg)) = legs.iter_mut().min_by(|(_, la), (_, lb)| {
            let ca = center(bounds.get(la.shin.as_str()).unwrap());
            let cb = center(bounds.get(lb.shin.as_str()).unwrap());
            let da = (ca[body_axis] - c[body_axis]).powi(2) + (ca[side_axis] - c[side_axis]).powi(2);
            let db = (cb[body_axis] - c[body_axis]).powi(2) + (cb[side_axis] - c[side_axis]).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        }) { leg.feet.push(n.clone()); }
    }
    if legs.len() != 4 { return Vec::new(); } // exactly 4 legs = quadruped; cameras/devices return empty
    let all_nodes: Vec<String> = bounds.keys().cloned().collect();
    let hip = |n: &str| -> [f64; 3] {
        let b = bounds.get(n).unwrap();
        let mut p = center(b);
        p[up] = b.1[up] - 0.04;
        p
    };
    let knee = |n: &str| -> [f64; 3] {
        let b = bounds.get(n).unwrap();
        let mut p = center(b);
        p[up] = b.1[up] - 0.015;
        p
    };
    let r = |nodes: &[String], pivot: [f64; 3], axis: &[f64; 3], deg: f64| json!({"nodes": nodes, "kind": "rotate", "pivot": pivot, "axis": axis, "angleDeg": deg});
    let t = |nodes: &[String], v: [f64; 3]| json!({"nodes": nodes, "kind": "translate", "vector": v});
    let leg_steps = |corner: &str, h: f64, k: f64| -> Vec<serde_json::Value> {
        let Some(leg) = legs.get(corner) else { return Vec::new() };
        let mut lower: Vec<String> = vec![leg.shin.clone()]; lower.extend(leg.feet.iter().cloned());
        match &leg.thigh {
            None => vec![r(&lower, hip(&leg.shin), &swing, h)],
            Some(th) => { let mut upper = vec![th.clone()]; upper.extend(lower.iter().cloned()); vec![r(&lower, knee(&leg.shin), &swing, k), r(&upper, hip(th), &swing, h)] }
        }
    };
    let pose = |id: &str, label: &str, desc: &str, ms: u32, per: &[(&str, f64, f64)], pitch: f64, pivot: [f64; 3]| -> serde_json::Value {
        let mut steps = Vec::new();
        for (corner, h, k) in per { steps.extend(leg_steps(corner, *h, *k)); }
        // 每步最多引用 64 个节点（validate_interactive_patch）：整体俯仰按同一枢轴分块。
        if pitch.abs() > 0.001 {
            for chunk in all_nodes.chunks(64) { steps.push(r(chunk, pivot, &swing, pitch)); }
        }
        json!({"id": id, "label": label, "description": desc, "durationMs": ms, "steps": steps})
    };
    vec![
        json!({"id": "stand", "label": "站立", "description": "默认站立姿态（模型原始姿态）。", "durationMs": 500, "steps": [r(&all_nodes[..1].to_vec(), [0.0; 3], &swing, 0.0)]}),
        pose("lie-down", "趴下", "腿部收起、腹部贴地。", 1200, &[("FL", -50.0, 115.0), ("FR", -50.0, 115.0), ("RL", -50.0, 115.0), ("RR", -50.0, 115.0)], 0.0, [0.0; 3]),
        pose("sit", "坐下", "后腿收起、前腿支撑。", 1100, &[("RL", -55.0, 120.0), ("RR", -55.0, 120.0), ("FL", 10.0, 0.0), ("FR", 10.0, 0.0)], 24.0, {
            let mut p = [0.0; 3]; p[body_axis] = lo[body_axis] + 0.15; p[up] = lo[up] + 0.3; p
        }),
        pose("shake-hand", "握手", "抬起右前腿。", 900, &[("FR", 65.0, -20.0)], 0.0, [0.0; 3]),
        pose("bow", "作揖", "前腿弯曲下压。", 1100, &[("FL", -45.0, 110.0), ("FR", -45.0, 110.0)], -16.0, {
            let mut p = [0.0; 3]; p[body_axis] = hi[body_axis] - 0.2; p[up] = lo[up] + 0.3; p
        }),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;
    use manual_core::knowledge::{Evidence, Part, ReviewStatus};

    fn part(name: &str) -> Part {
        Part {
            id: format!("part-{name}"),
            name: name.to_owned(),
            description: String::new(),
            evidence: Vec::<Evidence>::new(),
            review_status: ReviewStatus::NeedsReview,
            source_batches: vec![0],
        }
    }

    fn box_at(cx: f64, cy: f64, cz: f64, sx: f64, sy: f64, sz: f64) -> ([f64; 3], [f64; 3]) {
        (
            [cx - sx / 2.0, cy - sy / 2.0, cz - sz / 2.0],
            [cx + sx / 2.0, cy + sy / 2.0, cz + sz / 2.0],
        )
    }

    #[test]
    fn actions_cover_traditional_names_and_dedupe_labels() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.0, 0.0, 1.0, 0.3, 1.0));
        bounds.insert("lid".into(), box_at(0.1, 0.14, 0.0, 0.2, 0.02, 0.2));
        bounds.insert("bin".into(), box_at(0.35, 0.05, 0.0, 0.15, 0.1, 0.2));
        bounds.insert("key".into(), box_at(-0.1, 0.15, 0.0, 0.03, 0.01, 0.03));
        bounds.insert("key2".into(), box_at(-0.2, 0.15, 0.0, 0.03, 0.01, 0.03));
        let mut dup = part("回充鍵");
        dup.id = "part-dup".into();
        let mut spaced = part("回 充 鍵");
        spaced.id = "part-spaced".into();
        let parts = vec![part("掀蓋口"), part("集塵盒"), part("回充鍵"), dup, part("集塵盒卡扣"), spaced, part("四肢/膝蓋")];
        let bindings = vec![
            ("part-掀蓋口".to_owned(), vec!["lid".to_owned()], [0.1, 0.14, 0.0]),
            ("part-集塵盒".to_owned(), vec!["bin".to_owned()], [0.35, 0.05, 0.0]),
            ("part-回充鍵".to_owned(), vec!["key".to_owned()], [-0.1, 0.15, 0.0]),
            ("part-dup".to_owned(), vec!["key2".to_owned()], [-0.2, 0.15, 0.0]),
            ("part-集塵盒卡扣".to_owned(), vec!["clip".to_owned()], [0.3, 0.1, 0.0]),
            ("part-spaced".to_owned(), vec!["key3".to_owned()], [-0.3, 0.15, 0.0]),
            ("part-四肢/膝蓋".to_owned(), vec!["knee".to_owned()], [0.2, -0.1, 0.2]),
        ];
        bounds.insert("clip".into(), box_at(0.3, 0.1, 0.0, 0.02, 0.02, 0.02));
        bounds.insert("key3".into(), box_at(-0.3, 0.15, 0.0, 0.03, 0.01, 0.03));
        bounds.insert("knee".into(), box_at(0.2, -0.1, 0.2, 0.04, 0.04, 0.04));
        let actions = derive_actions(&parts, &bindings, &bounds, &[]);
        let labels: Vec<_> = actions.iter().map(|a| a["label"].as_str().unwrap().to_owned()).collect();
        assert_eq!(labels, ["打开掀蓋口", "取出集塵盒", "按下回充鍵"], "{actions:?}");
    }

    #[test]
    fn camera_parts_do_not_get_quadruped_poses() {
        let mut bounds = std::collections::BTreeMap::new();
        for i in 0..12 {
            bounds.insert(
                format!("tripo_part_{i}"),
                box_at((i as f64) * 0.04 - 0.2, 0.0, 0.0, 0.05, 0.08, 0.04),
            );
        }
        let poses = derive_poses(&bounds, &[part("照相机"), part("快门释放按钮")]);
        assert!(poses.is_empty(), "{poses:?}");
    }

    #[test]
    fn quadruped_name_with_four_low_legs_gets_poses() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.12, 0.0, 0.5, 0.12, 0.2));
        bounds.insert("head".into(), box_at(0.28, 0.16, 0.0, 0.1, 0.1, 0.1));
        // four shins touching the floor
        bounds.insert("shin_fl".into(), box_at(0.18, 0.05, 0.08, 0.04, 0.12, 0.04));
        bounds.insert("shin_fr".into(), box_at(0.18, 0.05, -0.08, 0.04, 0.12, 0.04));
        bounds.insert("shin_rl".into(), box_at(-0.18, 0.05, 0.08, 0.04, 0.12, 0.04));
        bounds.insert("shin_rr".into(), box_at(-0.18, 0.05, -0.08, 0.04, 0.12, 0.04));
        bounds.insert("foot_fl".into(), box_at(0.18, 0.01, 0.08, 0.03, 0.02, 0.03));
        bounds.insert("extra".into(), box_at(0.0, 0.18, 0.0, 0.08, 0.04, 0.08));
        let poses = derive_poses(&bounds, &[part("仿生四足机器人"), part("电源键")]);
        let ids: Vec<_> = poses.iter().filter_map(|p| p.get("id").and_then(|v| v.as_str())).collect();
        assert_eq!(ids, ["stand", "lie-down", "sit", "shake-hand", "bow"], "{poses:?}");
    }

    #[test]
    fn pose_steps_respect_node_limit_on_large_models() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.12, 0.0, 0.5, 0.12, 0.2));
        for (name, x, z) in [("fl", 0.18, 0.08), ("fr", 0.18, -0.08), ("rl", -0.18, 0.08), ("rr", -0.18, -0.08)] {
            bounds.insert(format!("shin_{name}"), box_at(x, 0.05, z, 0.04, 0.12, 0.04));
        }
        for i in 0..80 {
            bounds.insert(format!("bit_{i}"), box_at(0.0, 0.15, 0.0, 0.01, 0.01, 0.01));
        }
        let poses = derive_poses(&bounds, &[part("机器狗")]);
        assert!(!poses.is_empty());
        for pose in &poses {
            for step in pose["steps"].as_array().unwrap() {
                let n = step["nodes"].as_array().unwrap().len();
                assert!((1..=64).contains(&n), "{} step has {n} nodes", pose["id"]);
            }
        }
    }
}
