//! 分件模型附件的挂载（ADR-042）：`POST /items/{id}/drafts/{draftId}/parts-model`。
//!
//! 校验顺序（任一失败都不写入草稿）：
//! 1. 文件是合法 GLB，且通过与模型下载同一套预算（三角面/贴图/内嵌资源，见 `assets::glb`）；
//! 2. 节点名非空、可打印且唯一（绑定/动作只能引用这些名字）；
//! 3. **坐标系一致**：分件包围盒（含节点平移）与草稿模型的 `bounds` 在容差内一致——
//!    这是"分件是同一模型的拆分，不是另一个模型"的可验证证据；不一致 → 422；
//! 4. 草稿 CAS（If-Match）；成功后 `knowledge.interactive.partsModel` 指向新附件，
//!    旧绑定中引用不存在节点的条目被移除（不静默保留悬空引用）。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;
use sqlx::SqlitePool;

use manual_core::domain::{AssetPurpose, BlobStorageState};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

use super::interactive::{valid_node_name, Interactive, PartsModel};
use super::knowledge::DraftKnowledge;
use super::service::{read_draft, DraftServiceError};
use crate::assets::glb::{inspect_glb_file, GlbBudget};
use crate::storage::repo;
use crate::storage::repo::drafts as drafts_repo;

/// 包围盒一致性容差（模型已归一到约 ±0.5；分件重导出的浮点误差远小于此值）。
const BOUNDS_TOLERANCE: f64 = 0.02;
pub const AUDIT_PARTS_MODEL_ATTACHED: &str = "draft_parts_model_attached";

/// 读取 GLB 节点（名字 + 平移）；只接受无层级、无旋转/缩放的扁平节点（Tripo 分件形态）。
fn read_nodes(path: &Path) -> Result<(Vec<String>, Vec<[f64; 3]>, Vec<Option<usize>>), String> {
    let mut file = std::fs::File::open(path).map_err(|error| format!("读取文件失败：{error}"))?;
    let mut header = [0u8; 20];
    file.read_exact(&mut header).map_err(|_| "文件过短：不是 GLB".to_owned())?;
    let json_len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
    if &header[0..4] != b"glTF" || &header[16..20] != b"JSON" || json_len > 64 * 1024 * 1024 {
        return Err("不是合法的 GLB 容器".to_owned());
    }
    file.seek(SeekFrom::Start(20)).map_err(|error| error.to_string())?;
    let mut json = vec![0u8; json_len];
    file.read_exact(&mut json).map_err(|_| "GLB JSON chunk 截断".to_owned())?;
    let doc: Value = serde_json::from_slice(&json).map_err(|error| format!("GLB JSON 不可解析：{error}"))?;
    let nodes = doc.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut names = Vec::new();
    let mut translations = Vec::new();
    let mut meshes = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.get("children").is_some() || node.get("rotation").is_some() || node.get("scale").is_some() || node.get("matrix").is_some() {
            return Err(format!("节点 {index} 含层级或旋转/缩放：分件模型只接受扁平、仅平移的节点"));
        }
        let name = node.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
        let t = node.get("translation").and_then(Value::as_array);
        let translation = match t {
            None => [0.0; 3],
            Some(values) if values.len() == 3 => {
                let v: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
                if v.len() != 3 || v.iter().any(|x| !x.is_finite()) {
                    return Err(format!("节点 {index} 的平移不是有限数值"));
                }
                [v[0], v[1], v[2]]
            }
            Some(_) => return Err(format!("节点 {index} 的平移不是三元组")),
        };
        names.push(name);
        translations.push(translation);
        meshes.push(node.get("mesh").and_then(Value::as_u64).map(|m| m as usize));
    }
    Ok((names, translations, meshes))
}

/// 每个 mesh 的局部包围盒（读 POSITION accessor 的 min/max；glTF 规范要求 POSITION 带 min/max）。
fn mesh_bounds(path: &Path) -> Result<Vec<Option<([f64; 3], [f64; 3])>>, String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut header = [0u8; 20];
    file.read_exact(&mut header).map_err(|error| error.to_string())?;
    let json_len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
    let mut json = vec![0u8; json_len];
    file.read_exact(&mut json).map_err(|error| error.to_string())?;
    let doc: Value = serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    let accessors = doc.get("accessors").and_then(Value::as_array).cloned().unwrap_or_default();
    let read3 = |value: Option<&Value>| -> Option<[f64; 3]> {
        let a = value?.as_array()?;
        if a.len() != 3 { return None; }
        Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
    };
    let mut out = Vec::new();
    for mesh in doc.get("meshes").and_then(Value::as_array).cloned().unwrap_or_default() {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut any = false;
        for primitive in mesh.get("primitives").and_then(Value::as_array).cloned().unwrap_or_default() {
            let Some(index) = primitive.pointer("/attributes/POSITION").and_then(Value::as_u64) else { continue };
            let Some(accessor) = accessors.get(index as usize) else { continue };
            if let (Some(min), Some(max)) = (read3(accessor.get("min")), read3(accessor.get("max"))) {
                for k in 0..3 { lo[k] = lo[k].min(min[k]); hi[k] = hi[k].max(max[k]); }
                any = true;
            }
        }
        out.push(if any { Some((lo, hi)) } else { None });
    }
    Ok(out)
}

fn model_bounds(knowledge: &DraftKnowledge) -> Option<([f64; 3], [f64; 3])> {
    let bounds = knowledge.model.as_ref()?.bounds.as_ref()?;
    let read = |key: &str| -> Option<[f64; 3]> {
        let a = bounds.get(key)?.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    };
    Some((read("min")?, read("max")?))
}

pub struct AttachInput<'a> {
    pub item_id: &'a str,
    pub draft_id: &'a str,
    pub expected_revision: i64,
    pub staged: crate::assets::blob_store::Staged,
    pub source: String,
    pub actor: &'a str,
}

/// 校验 + 落盘 + 写草稿（调用方已把上传流写入 tmp 暂存文件）。
pub async fn attach_parts_model(
    pool: &SqlitePool,
    data_dir: &Path,
    input: AttachInput<'_>,
    now: Timestamp,
) -> Result<manual_core::domain::ManualDraft, DraftServiceError> {
    let staged = input.staged;
    let fail = |message: String| DraftServiceError::FieldIssues(vec![FieldIssue::new("file", message)]);
    let validation = (|| -> Result<(Vec<String>, ([f64; 3], [f64; 3])), String> {
        inspect_glb_file(&staged.path, &GlbBudget::default()).map_err(|error| format!("分件模型未通过 GLB 校验：{error}"))?;
        let (names, translations, meshes) = read_nodes(&staged.path)?;
        if names.is_empty() {
            return Err("分件模型没有任何节点".to_owned());
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in &names {
            if !valid_node_name(name) || !seen.insert(name.clone()) {
                return Err(format!("节点名为空、过长、含控制字符或重复：{name:?}"));
            }
        }
        let per_mesh = mesh_bounds(&staged.path)?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for (translation, mesh) in translations.iter().zip(meshes.iter()) {
            if let Some(Some((min, max))) = mesh.and_then(|m| per_mesh.get(m)) {
                for k in 0..3 {
                    lo[k] = lo[k].min(min[k] + translation[k]);
                    hi[k] = hi[k].max(max[k] + translation[k]);
                }
            }
        }
        if lo.iter().chain(hi.iter()).any(|v| !v.is_finite()) {
            return Err("无法计算分件模型包围盒（POSITION 缺少 min/max）".to_owned());
        }
        Ok((names, (lo, hi)))
    })();
    let (node_names, parts_bounds) = match validation {
        Ok(value) => value,
        Err(message) => {
            crate::assets::blob_store::discard_staged(staged).await;
            return Err(fail(message));
        }
    };

    let mut conn = pool.acquire().await?;
    let current = read_draft(&mut conn, input.item_id, input.draft_id).await?;
    let mut knowledge: DraftKnowledge = serde_json::from_value(current.knowledge_json.clone())
        .map_err(|error| DraftServiceError::Integrity { code: "draft_knowledge_unreadable", message: error.to_string() })?;
    let Some(model) = knowledge.model.clone() else {
        crate::assets::blob_store::discard_staged(staged).await;
        return Err(fail("草稿没有可用的模型版本：无法挂载分件模型".to_owned()));
    };
    let Some((model_lo, model_hi)) = model_bounds(&knowledge) else {
        crate::assets::blob_store::discard_staged(staged).await;
        return Err(fail("草稿模型缺少包围盒摘要：无法核对坐标系".to_owned()));
    };
    let mismatch = (0..3)
        .map(|k| (parts_bounds.0[k] - model_lo[k]).abs().max((parts_bounds.1[k] - model_hi[k]).abs()))
        .fold(0.0_f64, f64::max);
    if mismatch > BOUNDS_TOLERANCE {
        crate::assets::blob_store::discard_staged(staged).await;
        return Err(fail(format!(
            "分件模型与草稿模型的坐标系不一致（包围盒最大偏差 {mismatch:.4} > {BOUNDS_TOLERANCE}）：\
             请对**当前模型版本**做分件，而不是另一次生成的模型"
        )));
    }

    crate::assets::blob_store::promote(&staged, data_dir)
        .await
        .map_err(|error| DraftServiceError::Integrity { code: "parts_model_write_failed", message: format!("{error:?}") })?;

    let mut tx = crate::storage::begin_write(&mut conn).await?;
    let fresh = read_draft(&mut tx, input.item_id, input.draft_id).await?;
    if fresh.revision != input.expected_revision {
        tx.rollback().await?;
        return Err(DraftServiceError::Storage(crate::storage::StorageError::RevisionConflict {
            entity: "manual_draft",
            id: input.draft_id.to_owned(),
            current_revision: fresh.revision,
        }));
    }
    repo::blobs::insert_if_absent(&mut tx, &staged.sha256, staged.size, "model/gltf-binary").await?;
    if let Some(blob) = repo::blobs::get(&mut tx, &staged.sha256).await? {
        match blob.storage_state {
            BlobStorageState::Stored => {}
            BlobStorageState::Missing => {
                repo::blobs::set_storage_state(&mut tx, &staged.sha256, BlobStorageState::Stored).await?;
            }
            BlobStorageState::Quarantined => {
                tx.rollback().await?;
                return Err(fail("该文件内容与已隔离的 blob 相同：请管理员先处理隔离记录".to_owned()));
            }
        }
    }
    let asset = repo::assets::insert(
        &mut tx,
        repo::assets::NewAsset {
            blob_id: staged.sha256.clone(),
            item_id: input.item_id.to_owned(),
            purpose: AssetPurpose::ModelParts,
            original_name: None,
        },
    )
    .await?;

    let parts_model = PartsModel {
        asset_id: asset.id.clone(),
        sha256: staged.sha256.clone(),
        model_revision_id: model.revision_id.clone(),
        model_sha256: model.sha256.clone(),
        node_names: node_names.clone(),
        source: input.source.chars().take(120).collect(),
    };
    let names: std::collections::BTreeSet<&str> = node_names.iter().map(String::as_str).collect();
    let previous = knowledge.interactive.take();
    let mut interactive = Interactive { parts_model, bindings: Vec::new(), actions: Vec::new(), poses: Vec::new() };
    if let Some(previous) = previous {
        // 只保留仍然全部可解析的条目（不静默留下悬空节点引用）。
        interactive.bindings = previous.bindings.into_iter().filter(|b| b.nodes.iter().all(|n| names.contains(n.as_str()))).collect();
        interactive.actions = previous.actions.into_iter().filter(|a| a.steps.iter().all(|s| s.nodes.iter().all(|n| names.contains(n.as_str())))).collect();
        interactive.poses = previous.poses.into_iter().filter(|p| p.steps.iter().all(|s| s.nodes.iter().all(|n| names.contains(n.as_str())))).collect();
    }
    knowledge.interactive = Some(interactive);
    let knowledge_json = serde_json::to_string(&knowledge)
        .map_err(|error| DraftServiceError::Integrity { code: "draft_serialization_failed", message: error.to_string() })?;
    let updated = drafts_repo::update_content(
        &mut tx,
        input.draft_id,
        input.expected_revision,
        Some(&knowledge_json),
        None,
        fresh.status,
        now,
    )
    .await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "manual_draft".to_owned(),
            entity_id: updated.id.clone(),
            actor: Some(input.actor.to_owned()),
            action: AUDIT_PARTS_MODEL_ATTACHED.to_owned(),
            result: "ok".to_owned(),
            metadata_json: Some(
                serde_json::json!({ "assetId": asset.id, "sha256": staged.sha256, "nodes": node_names.len(), "revision": updated.revision })
                    .to_string(),
            ),
        },
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(updated)
}
