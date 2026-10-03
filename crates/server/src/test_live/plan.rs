use super::{
    LiveError, Result,
    contract::{Case, Mode},
};
use crate::{
    config::{CliOverrides, Settings, provider_overrides::ProviderConfigStore},
    generation::{estimate, jobs},
    http::dto::EstimateRequest,
    storage::repo,
};
use manual_core::{
    domain::{AssetPurpose, BlobStorageState},
    generation::{plan_manual_ai, sha256_hex},
    timestamps::Timestamp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};
use std::{fs, io::Read, path::Path};

pub struct Prepared {
    pub public: Value,
    pub quote: repo::quotes::QuoteRecord,
}
pub fn digest<T: serde::Serialize>(value: &T) -> String {
    sha256_hex(&serde_json::to_vec(value).expect("fixed JSON schema serializes"))
}
pub fn file_digest(path: &Path) -> Result<String> {
    let meta = fs::symlink_metadata(path).map_err(|_| LiveError::new("materialAsset"))?;
    if !meta.is_file() {
        return Err(LiveError::new("materialAsset"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|_| LiveError::new("materialAsset"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| LiveError::new("materialAsset"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub fn settings(case: &Case) -> Result<(Settings, ProviderConfigStore)> {
    let data = fs::canonicalize(&case.instance.data_dir).map_err(|_| LiveError::new("instance"))?;
    let config = fs::canonicalize(&case.instance.config_file)
        .map_err(|_| LiveError::new("configuration"))?;
    // Explicit instance-local configuration prevents cwd fallback into another library.
    if config.parent() != Some(data.as_path()) {
        return Err(LiveError::new("configuration"));
    }
    let mut settings = Settings::load(&CliOverrides {
        data_dir: Some(data.clone()),
        config: Some(config.clone()),
        listen: None,
    })
    .map_err(|_| LiveError::new("configuration"))?;
    if fs::canonicalize(&settings.data_dir).ok().as_ref() != Some(&data)
        || settings
            .config_path
            .as_ref()
            .and_then(|p| fs::canonicalize(p).ok())
            .as_ref()
            != Some(&config)
    {
        return Err(LiveError::new("instance"));
    }
    let store =
        ProviderConfigStore::load(&mut settings).map_err(|_| LiveError::new("configuration"))?;
    if settings.providers.tripo.model_issue() || settings.providers.manual_ai.model_issue() {
        return Err(LiveError::new("providerModelInvalid"));
    }
    store
        .ensure_generation_available()
        .map_err(|_| LiveError::new("configuration"))?;
    if !settings.providers.tripo.configured() || !settings.providers.manual_ai.configured() {
        return Err(LiveError::new("providerUnavailable"));
    }
    for provider in [&settings.providers.tripo, &settings.providers.manual_ai] {
        let url = reqwest::Url::parse(&provider.base_url)
            .map_err(|_| LiveError::new("providerEndpoint"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(LiveError::new("providerEndpoint"));
        }
        if case.mode == Mode::LoopbackFixture
            && (!matches!(url.host_str(), Some("127.0.0.1") | Some("[::1]"))
                || url.scheme() != "http")
        {
            return Err(LiveError::new("fixtureEndpoint"));
        }
        if case.mode == Mode::Real && url.scheme() != "https" {
            return Err(LiveError::new("providerEndpoint"));
        }
    }
    if case.mode == Mode::LoopbackFixture
        && (settings.download.allowed_hosts.is_empty()
            || settings
                .download
                .allowed_hosts
                .iter()
                .any(|h| h != "127.0.0.1" && h != "[::1]")
            || !settings.download.allow_local_fixture)
    {
        return Err(LiveError::new("fixtureEndpoint"));
    }
    Ok((settings, store))
}

async fn asset(
    conn: &mut SqliteConnection,
    settings: &Settings,
    item: &str,
    id: &str,
    purpose: AssetPurpose,
) -> Result<Value> {
    let (a, b) = repo::assets::get_with_blob(conn, id)
        .await
        .map_err(|_| LiveError::new("storage"))?
        .ok_or_else(|| LiveError::new("materialAsset"))?;
    if a.item_id != item
        || a.purpose != purpose
        || b.storage_state != BlobStorageState::Stored
        || !super::contract::hash(&b.sha256)
    {
        return Err(LiveError::new("materialAsset"));
    }
    if file_digest(&crate::assets::blob_path(&settings.data_dir, &b.sha256))? != b.sha256 {
        return Err(LiveError::new("materialAsset"));
    }
    Ok(json!({"assetId":id,"sha256":b.sha256,"size":b.size,"purpose":purpose.as_str()}))
}

pub async fn build(
    case: &Case,
    settings: &Settings,
    store: &ProviderConfigStore,
    pool: &SqlitePool,
    admin: &str,
    persist: bool,
) -> Result<Prepared> {
    let mut tx = crate::storage::begin_write_pool(pool)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let m = &case.material;
    let item = repo::items::get(&mut tx, &m.item_id)
        .await
        .map_err(|_| LiveError::new("storage"))?
        .ok_or_else(|| LiveError::new("materialIdentity"))?;
    let doc = repo::documents::get(&mut tx, &m.document_id)
        .await
        .map_err(|_| LiveError::new("storage"))?
        .ok_or_else(|| LiveError::new("materialIdentity"))?;
    let prep = repo::preparations::get(&mut tx, &m.preparation_id)
        .await
        .map_err(|_| LiveError::new("storage"))?
        .ok_or_else(|| LiveError::new("materialIdentity"))?;
    if item.model != m.item_model
        || doc.item_id != m.item_id
        || doc.source_sha256 != m.source_sha256
        || prep.document_id != doc.id
        || prep.source_sha256 != m.source_sha256
    {
        return Err(LiveError::new("materialIdentity"));
    }
    let source = asset(
        &mut tx,
        settings,
        &item.id,
        &doc.source_asset_id,
        AssetPurpose::Document,
    )
    .await?;
    if source["sha256"] != m.source_sha256 {
        return Err(LiveError::new("materialIdentity"));
    }
    let ids: Vec<_> = m.photos.iter().map(|p| p.id.clone()).collect();
    let photos = estimate::load_photos_for_item(&mut tx, &item.id, &ids)
        .await
        .map_err(|_| LiveError::new("materialIdentity"))?;
    for photo in &photos {
        if !m.photos.iter().any(|p| {
            p.id == photo.photo.id
                && p.sha256 == photo.sha256
                && p.view == photo.photo.view.as_str()
        }) {
            return Err(LiveError::new("materialIdentity"));
        }
        asset(
            &mut tx,
            settings,
            &item.id,
            &photo.photo.asset_id,
            AssetPurpose::Photo,
        )
        .await?;
    }
    let quote = estimate::create_estimate_with_revision(
        settings,
        &mut tx,
        &item.id,
        &EstimateRequest {
            preparation_id: Some(prep.id.clone()),
            photo_ids: Some(ids),
            model_preset: Some(case.generation.model_preset.clone()),
        },
        Timestamp::now(),
        Some(store.revision()),
    )
    .await
    .map_err(|error| LiveError::from_generation(&error))?;
    let payload = estimate::quote_payload(&quote)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    if payload.provider_config.tripo.model != case.generation.tripo.model
        || payload.provider_config.manual_ai.model != case.generation.manual_ai.model
        || payload.price_version != case.generation.price_version
    {
        return Err(LiveError::new("generationIdentity"));
    }
    let input_pages = repo::preparations::page_quote_inputs(&mut tx, &prep.id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let batches = plan_manual_ai(&input_pages).map_err(|_| LiveError::new("materialIdentity"))?;
    let stages = jobs::build_stage_plan("", &batches, &quote, &photos, false);
    let pages = repo::preparations::list_pages(&mut tx, &prep.id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let mut page_facts = Vec::new();
    for page in pages {
        let text = match page.text_asset_id {
            Some(id) => {
                Some(asset(&mut tx, settings, &item.id, &id, AssetPurpose::PageText).await?)
            }
            None => None,
        };
        let image = match page.image_asset_id {
            Some(id) => {
                Some(asset(&mut tx, settings, &item.id, &id, AssetPurpose::PageImage).await?)
            }
            None => None,
        };
        page_facts.push(
            json!({"page":page.page_number,"text":text,"image":image,"viewport":page.viewport}),
        );
    }
    let catalog = settings
        .price_catalog_path
        .as_ref()
        .ok_or_else(|| LiveError::new("priceCatalog"))?;
    let config_hash = digest(
        &json!({"tripoEndpoint":settings.providers.tripo.base_url.trim_end_matches('/'),"manualAiEndpoint":settings.providers.manual_ai.base_url.trim_end_matches('/'),"providers":payload.provider_config,"credentialBinding":digest(&json!([case.instance.instance_id,settings.providers.tripo.api_key.as_ref().map(|k|k.expose()),settings.providers.manual_ai.api_key.as_ref().map(|k|k.expose())])),"catalogSha256":file_digest(catalog)?,"downloadHosts":settings.download.allowed_hosts,"fixtureDownload":settings.download.allow_local_fixture,"maxGlbBytes":settings.limits.max_glb_bytes}),
    );
    let instance_hash = digest(
        &json!({"dataDir":fs::canonicalize(&settings.data_dir).map_err(|_|LiveError::new("instance"))?,"configFile":fs::canonicalize(&case.instance.config_file).map_err(|_|LiveError::new("instance"))?,"adminId":admin}),
    );
    let mut public = json!({
        "schemaVersion":1,
        "mode":case.mode,
        "caseId":case.case_id,
        "caseHash":digest(case),
        "instanceId":case.instance.instance_id,
        "instanceHash":instance_hash,
        "itemId":item.id,
        "itemModel":m.item_model,
        "documentId":doc.id,
        "sourceSha256":source["sha256"],
        "preparationId":prep.id,
        "inputHash":quote.input_hash,
        "pageRange":payload.page_range,
        "pages":page_facts,
        "batches":batches.batches,
        "textPages":batches.text_pages,
        "imagePages":batches.image_pages,
        "inputTokensUpperBound":batches.input_tokens_upper_bound,
        "maxOutputTokens":payload.max_output_tokens,
        "views":payload.send_scope.tripo.views,
        "modelPreset":payload.send_scope.tripo.preset,
        "parameters":payload.send_scope.tripo.parameters,
        "providers":payload.provider_config,
        "priceVersion":payload.price_version,
        "priceSnapshotDate":payload.price_snapshot_date,
        "configurationHash":config_hash,
        "requiredLimits":{"creditMinor":payload.amounts.tripo.upper_bound_minor,"usdMicros":payload.amounts.manual_ai.upper_bound_minor},
        "maxInitialGenerations":1,
        "stages":stages.iter().map(|s|json!({"stageKind":s.stage_kind.as_str(),"batchIndex":s.batch_index,"stageInputHash":s.input_hash})).collect::<Vec<_>>()
    });
    let hash = digest(&public);
    public["planHash"] = json!(hash);
    if persist {
        tx.commit().await.map_err(|_| LiveError::new("storage"))?;
    } else {
        tx.rollback().await.map_err(|_| LiveError::new("storage"))?;
    }
    Ok(Prepared { public, quote })
}
