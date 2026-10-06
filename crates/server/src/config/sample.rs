//! Built-in CyberDog 2 sample: seeded into a fresh data-dir on `init` so the
//! library is not empty on first login. The sample includes a published release
//! with a 6-part 3D model (head, torso, 4 legs), 5 poses, 7 steps and 8 specs.
//!
//! Assets are embedded at compile time via `include_bytes!`. The seed runs inside
//! a single write transaction after the admin row is created; if the item table
//! is already non-empty the seed is skipped (re-running `init` on an existing
//! data-dir does not duplicate it).

use std::path::Path;

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::StorageError;

const MODEL_GLB: &[u8] = include_bytes!("../../sample/cyberdog2/model.glb");
const MANUAL_PDF: &[u8] = include_bytes!("../../sample/cyberdog2/manual.pdf");
const KNOWLEDGE_TEMPLATE: &str = include_str!("../../sample/cyberdog2/knowledge.json");
const REVIEW_TEMPLATE: &str = include_str!("../../sample/cyberdog2/review.json");

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Write a blob file to the data-dir blob store (content-addressed, idempotent).
fn write_blob(data_dir: &Path, sha: &str, data: &[u8]) -> Result<(), std::io::Error> {
    let dir = data_dir.join("blobs").join(&sha[..2]);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(sha);
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Seed the CyberDog 2 sample into a fresh database. Skips silently if items already exist.
pub async fn seed(
    conn: &mut SqliteConnection,
    data_dir: &Path,
    admin_id: &str,
) -> Result<bool, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
        .fetch_one(&mut *conn)
        .await?;
    if count > 0 {
        return Ok(false);
    }

    let now = Timestamp::now();
    let ms = now.as_millis();

    // --- IDs ---
    let item_id = ids::new_id();
    let model_blob_sha = sha256_hex(MODEL_GLB);
    let pdf_blob_sha = sha256_hex(MANUAL_PDF);
    let model_asset_id = ids::new_id();
    let parts_asset_id = ids::new_id(); // same GLB serves as both model and parts
    let pdf_asset_id = ids::new_id();
    let manifest_asset_id = ids::new_id();
    let document_id = ids::new_id();
    let preparation_id = ids::new_id();
    let snapshot_id = ids::new_id();
    let model_revision_id = ids::new_id();
    let draft_id = ids::new_id();
    let release_id = ids::new_id();

    // --- Write blob files ---
    write_blob(data_dir, &model_blob_sha, MODEL_GLB)
        .map_err(|e| StorageError::Database { detail: format!("写入示例 GLB 失败：{e}") })?;
    write_blob(data_dir, &pdf_blob_sha, MANUAL_PDF)
        .map_err(|e| StorageError::Database { detail: format!("写入示例 PDF 失败：{e}") })?;

    // --- Template substitution ---
    let knowledge_json = KNOWLEDGE_TEMPLATE
        .replace("{{DOC}}", &document_id)
        .replace("{{PREP}}", &preparation_id)
        .replace("{{MODEL_REV}}", &model_revision_id)
        .replace("{{MODEL_SHA}}", &model_blob_sha)
        .replace("{{MODEL_ASSET}}", &model_asset_id)
        .replace("{{PARTS_ASSET}}", &parts_asset_id);
    let review_json = REVIEW_TEMPLATE
        .replace("{{MODEL_REV}}", &model_revision_id)
        .replace("{{MODEL_SHA}}", &model_blob_sha);

    // Verify JSON is valid before inserting
    let _: serde_json::Value = serde_json::from_str(&knowledge_json)
        .map_err(|e| StorageError::Database { detail: format!("示例 knowledge JSON 无效：{e}") })?;
    let _: serde_json::Value = serde_json::from_str(&review_json)
        .map_err(|e| StorageError::Database { detail: format!("示例 review JSON 无效：{e}") })?;

    // --- Build manifest ---
    let manifest = serde_json::json!({
        "schemaVersion": "manual_release_v1",
        "releaseId": release_id,
        "itemId": item_id,
        "draftId": draft_id,
        "draftRevision": 2,
        "publishedAt": now.to_rfc3339(),
        "model": {
            "revisionId": model_revision_id,
            "sha256": model_blob_sha,
            "assetId": model_asset_id,
            "validationState": "validated",
            "bounds": serde_json::from_str::<serde_json::Value>(&knowledge_json)
                .unwrap()["model"]["bounds"].clone(),
        },
        "knowledge": serde_json::from_str::<serde_json::Value>(&knowledge_json).unwrap(),
        "review": serde_json::from_str::<serde_json::Value>(&review_json).unwrap(),
        "assets": [
            {"assetId": model_asset_id, "role": "model", "sha256": model_blob_sha, "size": MODEL_GLB.len(), "source": "builtin-sample"},
            {"assetId": parts_asset_id, "role": "model_parts", "sha256": model_blob_sha, "size": MODEL_GLB.len(), "source": "builtin-sample"},
            {"assetId": pdf_asset_id, "role": "document", "sha256": pdf_blob_sha, "size": MANUAL_PDF.len(), "source": "builtin-sample"},
        ],
        "documents": [{
            "documentId": document_id,
            "preparationId": preparation_id,
            "sourceAssetId": pdf_asset_id,
            "sourceSha256": pdf_blob_sha,
            "sourceUrl": null,
            "title": "CyberDog 2 机器狗示例说明书",
        }],
        "counts": {
            "confirmedHotspots": 6, "hotspots": 6, "parts": 6, "specs": 8, "steps": 7,
            "textOnlyParts": 0, "textOnlyPartIds": [],
        },
    });
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|e| StorageError::Database { detail: format!("序列化 manifest 失败：{e}") })?;
    let manifest_sha = sha256_hex(&manifest_bytes);
    write_blob(data_dir, &manifest_sha, &manifest_bytes)
        .map_err(|e| StorageError::Database { detail: format!("写入示例 manifest 失败：{e}") })?;

    // --- SQL inserts (single transaction, caller wraps BEGIN IMMEDIATE) ---

    // item
    sqlx::query("INSERT INTO items (id, name, brand, model, variant, revision, created_at, updated_at) VALUES (?, '机器狗', '小米', 'CyberDog 2', NULL, 1, ?, ?)")
        .bind(&item_id).bind(ms).bind(ms).execute(&mut *conn).await?;

    // blobs
    for (sha, size, mime) in [(&model_blob_sha, MODEL_GLB.len(), "model/gltf-binary"), (&pdf_blob_sha, MANUAL_PDF.len(), "application/pdf"), (&manifest_sha, manifest_bytes.len(), "application/json")] {
        sqlx::query("INSERT OR IGNORE INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, ?, ?, 'stored', ?)")
            .bind(sha).bind(size as i64).bind(mime).bind(ms).execute(&mut *conn).await?;
    }

    // assets
    for (aid, sha, purpose, name) in [
        (&model_asset_id, &model_blob_sha, "model", Some("cyberdog2-sample.glb")),
        (&parts_asset_id, &model_blob_sha, "model_parts", Some("cyberdog2-sample-parts.glb")),
        (&pdf_asset_id, &pdf_blob_sha, "document", Some("cyberdog2-sample-manual.pdf")),
        (&manifest_asset_id, &manifest_sha, "release_manifest", None),
    ] {
        sqlx::query("INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(aid).bind(sha).bind(&item_id).bind(purpose).bind(name).bind(ms).execute(&mut *conn).await?;
    }

    // document
    sqlx::query("INSERT INTO documents (id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at) VALUES (?, ?, ?, ?, 'CyberDog 2 机器狗示例说明书', NULL, ?, ?)")
        .bind(&document_id).bind(&item_id).bind(&pdf_asset_id).bind(&pdf_blob_sha).bind(ms).bind(ms).execute(&mut *conn).await?;

    // preparation (ready, 4 pages)
    sqlx::query("INSERT INTO preparations (id, document_id, source_sha256, state, page_count, client_derived, revision, created_at, updated_at) VALUES (?, ?, ?, 'ready', 4, 0, 1, ?, ?)")
        .bind(&preparation_id).bind(&document_id).bind(&pdf_blob_sha).bind(ms).bind(ms).execute(&mut *conn).await?;

    // generation_snapshot (minimal; draft FK requires it)
    sqlx::query("INSERT INTO generation_snapshots (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, provider_config, prompt_version, price_version, budgets, created_at) VALUES (?, ?, 1, ?, '[]', '[]', '{\"builtin\":true}', 'builtin', 'builtin', '{\"builtin\":true}', ?)")
        .bind(&snapshot_id).bind(&item_id).bind(&preparation_id).bind(ms).execute(&mut *conn).await?;

    // model_revision
    sqlx::query("INSERT INTO model_revisions (id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at) VALUES (?, ?, ?, ?, NULL, ?, 'validated', ?)")
        .bind(&model_revision_id).bind(&item_id).bind(&model_asset_id).bind(&model_blob_sha)
        .bind(serde_json::to_string(&manifest["model"]["bounds"]).unwrap())
        .bind(ms).execute(&mut *conn).await?;

    // draft (revision 2 = after publish bumps it)
    sqlx::query("INSERT INTO manual_drafts (id, item_id, snapshot_id, model_revision_id, revision, status, knowledge_json, review_json, created_at, updated_at) VALUES (?, ?, ?, ?, 2, 'needs_review', ?, ?, ?, ?)")
        .bind(&draft_id).bind(&item_id).bind(&snapshot_id).bind(&model_revision_id)
        .bind(&knowledge_json).bind(&review_json).bind(ms).bind(ms).execute(&mut *conn).await?;

    // release
    sqlx::query("INSERT INTO manual_releases (id, item_id, draft_id, draft_revision, model_revision_id, manifest_asset_id, created_at) VALUES (?, ?, ?, 2, ?, ?, ?)")
        .bind(&release_id).bind(&item_id).bind(&draft_id).bind(&model_revision_id).bind(&manifest_asset_id).bind(ms).execute(&mut *conn).await?;

    // audit event
    sqlx::query("INSERT INTO audit_events (id, entity_type, entity_id, actor, action, result, metadata_json, created_at) VALUES (?, 'manual_release', ?, ?, 'builtin_sample_seeded', 'ok', NULL, ?)")
        .bind(ids::new_id()).bind(&release_id).bind(admin_id).bind(ms).execute(&mut *conn).await?;

    tracing::info!(
        event = "sample_seeded",
        itemId = %item_id,
        releaseId = %release_id,
        "内置示例「机器狗」已写入：包含 6 部件热点、5 个姿势、7 步骤、8 规格与已发布版本"
    );

    Ok(true)
}
