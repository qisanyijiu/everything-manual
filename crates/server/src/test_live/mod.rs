//! Budget-file-controlled local verification. Reuses the production quote,
//! frozen job, submission window and ledger; it never reviews or publishes.
pub mod contract;
mod plan;
mod report;

use crate::{
    config::{
        datadir,
        encrypted_secrets::{read_private_file, write_private_file},
    },
    generation::{GenerationError, estimate, jobs},
    http::dto::{BudgetLimitsDto, JobCreateRequest},
    jobs::{
        ExecutorConfig, JobExecutor, PipelineHandlers, StageRegistry,
        scope::{CallGate, CallGateFuture, ExecutionScope},
    },
    storage::{Database, repo},
};
use contract::{Budget, Case, Mode};
use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub type Result<T> = std::result::Result<T, LiveError>;
#[derive(Debug, Clone)]
pub struct LiveError {
    pub code: &'static str,
    pub execution_started: bool,
}
impl LiveError {
    pub fn new(code: &'static str) -> Self {
        Self {
            code,
            execution_started: false,
        }
    }
    fn from_generation(error: &GenerationError) -> Self {
        Self::new(match error {
            GenerationError::ProviderNotConfigured { .. } => "providerUnavailable",
            GenerationError::PriceCatalogMissing { .. } => "priceCatalog",
            GenerationError::Unprocessable {
                reason: "quoteExpired",
                ..
            } => "quoteExpired",
            GenerationError::Unprocessable {
                reason: "providerModelInvalid" | "quoteModelInvalid",
                ..
            } => "providerModelInvalid",
            GenerationError::Storage(_) => "storage",
            _ => "generationPreconditions",
        })
    }
    pub fn message(&self) -> &'static str {
        match self.code {
            "budgetRequired" => "执行需要 --budget-file；只核对计划请使用 --plan。",
            "caseFile" => "案例必须是可读取的有界普通JSON文件。",
            "caseJson" | "caseFields" => {
                "案例字段缺失、未知或无效；按 test-live 文档修正字段，不要写入凭据。"
            }
            "budgetFilePermissions" => {
                "授权必须是受限普通文件（Unix权限0600）；拒绝软链接和公开权限。"
            }
            "budgetJson" | "budgetFields" => {
                "授权字段无效；上限须为非负整数，两币种独立，初始生成次数必须为1。"
            }
            "notAuthorized" => "授权未明确 allowed=true；请由授权人核对计划。",
            "authorizationExpired" => "授权已过期；保留原任务，重新核对授权，不删除执行记录。",
            "authorizationChanged" | "planChanged" => {
                "授权、实例或绑定计划已变化；本次不执行，核对原授权与现有任务。"
            }
            "creditBudget" => "creditMinor 上限不足；核对该币种的计划上界。",
            "usdBudget" => "usdMicros 上限不足；核对该币种的计划上界。",
            "retryScope" => {
                "重试必须绑定具体付费阶段、批次和输入hash，最多5次仅限明确未受理安全重试；unknown不允许重购。"
            }
            "instanceBusy" => "专用实例正被另一进程使用；等待后用同一授权恢复，不另建业务键。",
            "instance" => "需要已初始化的专用隔离实例；停服后核对实例目录。",
            "configuration" | "providerUnavailable" => {
                "现有安全配置不可用；在专用实例设置供应商并重启核对，不把密钥放入命令或案例。"
            }
            "providerModelInvalid" => "模型栏疑似误填密钥；通过安全设置入口修正模型。",
            "priceCatalog" => "缺少可靠价格目录或所选模型价格；不可估的费用不按0执行。",
            "materialIdentity"
            | "materialAsset"
            | "generationIdentity"
            | "generationPreconditions" => {
                "所选资料、已准备页或生成配置与案例不符；核对原件hash、照片视图及模型。"
            }
            "fixtureEndpoint" => "fixture仅允许字面loopback供应商与下载地址；不能用于公网。",
            "fixtureBuild" => "fixture执行需要显式测试构建；普通发行构建不放行本机模型下载。",
            "providerEndpoint" => {
                "供应商地址必须是不含凭据或查询参数的有效地址；真实模式要求HTTPS。"
            }
            "quoteExpired" => {
                "冻结报价在建单前已过期；保留原授权记录，重新核对计划，不自动更换报价或业务键。"
            }
            "journal" => "授权执行记录不可用或不一致；保留现有任务与文件，检查专用实例。",
            "output" => "结果目录必须独立于实例，且只含命令创建的受限结果文件；检查写入权限。",
            _ => "本地读取或执行未完成；保留原授权与已有成果，核对后使用同一授权恢复。",
        }
    }
}
#[derive(Clone)]
pub struct RunOptions {
    pub case: PathBuf,
    pub budget_file: Option<PathBuf>,
    pub plan: bool,
}
pub struct RunResult {
    pub success: bool,
    pub title: &'static str,
    pub data: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    authorization_hash: String,
    case_hash: String,
    plan_hash: String,
    actor_id: String,
    key: String,
    request: JobCreateRequest,
    job_id: Option<String>,
    plan: Value,
}

fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path).map_err(|_| LiveError::new("output"))?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| LiveError::new("output"))?;
    if !metadata.is_dir() {
        return Err(LiveError::new("output"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(LiveError::new("output"));
        }
    }
    Ok(())
}
fn save_journal(path: &Path, journal: &Journal, replace: bool) -> Result<()> {
    write_private_file(
        path,
        &serde_json::to_vec(journal).map_err(|_| LiveError::new("journal"))?,
        replace,
    )
    .map_err(|_| LiveError::new("journal"))
}
fn authorize(case: &Case, budget: &Budget, plan: &Value) -> Result<()> {
    budget.validate()?;
    if budget.case_id != case.case_id
        || budget.case_hash != plan::digest(case)
        || plan["planHash"] != budget.plan_hash
    {
        return Err(LiveError::new("planChanged"));
    }
    if budget.limits.credit_minor
        < plan["requiredLimits"]["creditMinor"]
            .as_i64()
            .ok_or_else(|| LiveError::new("priceCatalog"))?
    {
        return Err(LiveError::new("creditBudget"));
    }
    if budget.limits.usd_micros
        < plan["requiredLimits"]["usdMicros"]
            .as_i64()
            .ok_or_else(|| LiveError::new("priceCatalog"))?
    {
        return Err(LiveError::new("usdBudget"));
    }
    for scope in &budget.retry_scopes {
        if !plan["stages"].as_array().is_some_and(|stages| {
            stages.iter().any(|s| {
                s["stageKind"] == scope.stage_kind
                    && s["batchIndex"] == scope.batch_index
                    && s["stageInputHash"] == scope.stage_input_hash
            })
        }) {
            return Err(LiveError::new("retryScope"));
        }
    }
    Ok(())
}

struct Gate {
    case: Case,
    budget_file: PathBuf,
    authorization_hash: String,
    plan_hash: String,
    admin: String,
    pool: sqlx::SqlitePool,
    job_id: std::sync::Mutex<Option<String>>,
}
impl CallGate for Gate {
    fn check(&self) -> CallGateFuture<'_> {
        Box::pin(async move {
            let result: Result<()> = async {
                let budget = Budget::read(&self.budget_file)?;
                if plan::digest(&budget) != self.authorization_hash {
                    return Err(LiveError::new("authorizationChanged"));
                }
                let (settings, store) = plan::settings(&self.case)?;
                let prepared = plan::build(
                    &self.case,
                    &settings,
                    &store,
                    &self.pool,
                    &self.admin,
                    false,
                )
                .await?;
                if prepared.public["planHash"] != self.plan_hash {
                    return Err(LiveError::new("planChanged"));
                }
                authorize(&self.case, &budget, &prepared.public)?;
                let job_id = self
                    .job_id
                    .lock()
                    .map_err(|_| LiveError::new("journal"))?
                    .clone();
                if let Some(job_id) = job_id {
                    let mut conn = self
                        .pool
                        .acquire()
                        .await
                        .map_err(|_| LiveError::new("storage"))?;
                    let job = repo::jobs::get(&mut conn, &job_id)
                        .await
                        .map_err(|_| LiveError::new("storage"))?
                        .ok_or_else(|| LiveError::new("journal"))?;
                    let stages = repo::job_stages::list_for_job(&mut conn, &job_id)
                        .await
                        .map_err(|_| LiveError::new("storage"))?;
                    let expected = prepared.public["stages"]
                        .as_array()
                        .ok_or_else(|| LiveError::new("journal"))?;
                    if stages.len() != expected.len()
                        || stages.iter().any(|stage| {
                            !expected.iter().any(|e| {
                                e["stageKind"] == stage.stage_kind.as_str()
                                    && e["batchIndex"] == stage.batch_index
                                    && e["stageInputHash"] == stage.input_hash
                            })
                        })
                    {
                        return Err(LiveError::new("stageScopeChanged"));
                    }
                    let entries = repo::ledger::list_for_snapshot(&mut conn, &job.snapshot_id)
                        .await
                        .map_err(|_| LiveError::new("storage"))?;
                    let mut actual = [0_i64; 2];
                    for entry in entries {
                        if let Some(value) = entry.actual {
                            let index =
                                if entry.currency == manual_core::domain::Currency::CreditMinor {
                                    0
                                } else {
                                    1
                                };
                            actual[index] = actual[index]
                                .checked_add(value)
                                .ok_or_else(|| LiveError::new("budgetRisk"))?;
                        }
                    }
                    if actual[0] > budget.limits.credit_minor
                        || actual[1] > budget.limits.usd_micros
                    {
                        return Err(LiveError::new("budgetRisk"));
                    }
                }
                Ok(())
            }
            .await;
            result.map_err(|error| error.code)
        })
    }
}

pub async fn run(options: RunOptions) -> Result<RunResult> {
    let started = std::sync::atomic::AtomicBool::new(false);
    run_inner(options, &started).await.map_err(|mut error| {
        error.execution_started = started.load(std::sync::atomic::Ordering::SeqCst);
        error
    })
}
async fn run_inner(
    options: RunOptions,
    started: &std::sync::atomic::AtomicBool,
) -> Result<RunResult> {
    // Authorization syntax/permissions/time gates occur before opening the DB,
    // loading provider clients, or touching another job's recovery state.
    let budget = if options.plan {
        None
    } else {
        Some(Budget::read(
            options
                .budget_file
                .as_deref()
                .ok_or_else(|| LiveError::new("budgetRequired"))?,
        )?)
    };
    let case = Case::read(&options.case)?;
    let (settings, store) = plan::settings(&case)?;
    datadir::verify(&settings.data_dir).map_err(|_| LiveError::new("instance"))?;
    let _lock = datadir::DirLock::acquire(&settings.data_dir)
        .map_err(|_| LiveError::new("instanceBusy"))?;
    let database = Database::open_existing(&settings.data_dir)
        .await
        .map_err(|_| LiveError::new("instance"))?
        .ok_or_else(|| LiveError::new("instance"))?;
    let admin = {
        let mut conn = database
            .pool()
            .acquire()
            .await
            .map_err(|_| LiveError::new("storage"))?;
        repo::admins::get_single(&mut conn)
            .await
            .map_err(|_| LiveError::new("storage"))?
            .ok_or_else(|| LiveError::new("instance"))?
            .id
    };
    let prepared = plan::build(&case, &settings, &store, database.pool(), &admin, false).await?;
    if options.plan {
        return Ok(RunResult {
            success: true,
            title: "计划预览 · 未执行（未发起供应商请求）",
            data: prepared.public,
        });
    }
    let budget = budget.expect("execution parsed budget");
    authorize(&case, &budget, &prepared.public)?;
    if case.mode == Mode::LoopbackFixture && !crate::assets::glb::download::TEST_BUILD {
        return Err(LiveError::new("fixtureBuild"));
    }
    let output = fs::canonicalize(&case.output_directory).map_err(|_| LiveError::new("output"))?;
    let data = fs::canonicalize(&settings.data_dir).map_err(|_| LiveError::new("instance"))?;
    if output.starts_with(&data) || data.starts_with(&output) {
        return Err(LiveError::new("output"));
    }
    let output = output.join(format!(
        "{}-{}",
        case.case_id,
        &plan::digest(&budget.authorization_id)[..16]
    ));
    private_dir(&output)?;
    let journal_dir = data.join("live-authorizations");
    private_dir(&journal_dir)?;
    let path = journal_dir.join(format!("{}.json", plan::digest(&budget.authorization_id)));
    let authorization_hash = plan::digest(&budget);
    let initial_key = format!(
        "test-live-{}",
        plan::digest(&json!([
            prepared.public["instanceHash"],
            budget.authorization_id,
            "initial"
        ]))
    );
    let existed = fs::symlink_metadata(&path).is_ok();
    let mut journal = if existed {
        let raw = read_private_file(&path).map_err(|_| LiveError::new("journal"))?;
        let j: Journal = serde_json::from_slice(&raw).map_err(|_| LiveError::new("journal"))?;
        if j.schema_version != 1
            || j.authorization_hash != authorization_hash
            || j.case_hash != budget.case_hash
            || j.plan_hash != budget.plan_hash
            || j.actor_id != admin
            || j.key != initial_key
            || j.plan != prepared.public
            || j.request.preparation_id.as_ref() != Some(&case.material.preparation_id)
            || j.request.photo_ids.as_ref() != Some(&prepared.quote.photo_ids)
            || j.request.limits.as_ref().is_none_or(|l| {
                l.tripo_credit_minor != Some(budget.limits.credit_minor)
                    || l.manual_ai_usd_micros != Some(budget.limits.usd_micros)
            })
        {
            return Err(LiveError::new("authorizationChanged"));
        }
        j
    } else {
        // Persist only after all file, material, independent budget and output gates.
        let persisted =
            plan::build(&case, &settings, &store, database.pool(), &admin, true).await?;
        authorize(&case, &budget, &persisted.public)?;
        let j = Journal {
            schema_version: 1,
            authorization_hash: authorization_hash.clone(),
            case_hash: budget.case_hash.clone(),
            plan_hash: budget.plan_hash.clone(),
            actor_id: admin.clone(),
            key: initial_key,
            request: JobCreateRequest {
                quote_id: Some(persisted.quote.id),
                preparation_id: Some(case.material.preparation_id.clone()),
                photo_ids: Some(persisted.quote.photo_ids),
                limits: Some(BudgetLimitsDto {
                    tripo_credit_minor: Some(budget.limits.credit_minor),
                    manual_ai_usd_micros: Some(budget.limits.usd_micros),
                }),
            },
            job_id: None,
            plan: persisted.public,
        };
        save_journal(&path, &j, false)?;
        j
    };
    let gate = Arc::new(Gate {
        case: case.clone(),
        budget_file: options.budget_file.expect("budget path"),
        authorization_hash,
        plan_hash: budget.plan_hash.clone(),
        admin: admin.clone(),
        pool: database.pool().clone(),
        job_id: std::sync::Mutex::new(None),
    });
    gate.check().await.map_err(LiveError::new)?;
    let creation = {
        let mut conn = database
            .pool()
            .acquire()
            .await
            .map_err(|_| LiveError::new("storage"))?;
        // Replay first: an accepted job remains findable even after quote expiry.
        let quote = repo::quotes::get(
            &mut conn,
            journal
                .request
                .quote_id
                .as_deref()
                .ok_or_else(|| LiveError::new("journal"))?,
        )
        .await
        .map_err(|_| LiveError::new("journal"))?
        .ok_or_else(|| LiveError::new("journal"))?;
        if !quote.is_consumed() {
            estimate::confirm_quote(
                &mut conn,
                &case.material.item_id,
                &quote.id,
                &admin,
                Timestamp::now(),
            )
            .await
            .map_err(|e| LiveError::from_generation(&e))?;
        }
        jobs::create_job_with_config(
            &settings,
            &mut conn,
            &case.material.item_id,
            &journal.request,
            &journal.key,
            &admin,
            Timestamp::now(),
            Some(&store),
        )
        .await
        .map_err(|e| LiveError::from_generation(&e))?
    };
    if journal
        .job_id
        .as_ref()
        .is_some_and(|id| id != &creation.job.id)
    {
        return Err(LiveError::new("journal"));
    }
    journal.job_id = Some(creation.job.id.clone());
    save_journal(&path, &journal, true)?;
    *gate.job_id.lock().map_err(|_| LiveError::new("journal"))? = Some(creation.job.id.clone());
    let gate: Arc<dyn CallGate> = gate;
    println!(
        "授权计划已冻结 · {} · job {}",
        if existed {
            "恢复已有任务"
        } else {
            "已受理"
        },
        creation.job.id
    );
    let mut registry = StageRegistry::new();
    gate.check().await.map_err(LiveError::new)?;
    crate::providers::register_provider_handlers(&mut registry, &settings)
        .map_err(|_| LiveError::new("configuration"))?;
    PipelineHandlers::from_settings(&settings).register(&mut registry, &settings);
    let limits: BTreeMap<_, _> = budget
        .retry_scopes
        .iter()
        .map(|s| {
            (
                (
                    s.stage_kind.clone(),
                    s.batch_index,
                    s.stage_input_hash.clone(),
                ),
                s.max_additional_attempts,
            )
        })
        .collect();
    let executor = JobExecutor::with_scope(
        JobExecutor::with_provider_config(
            database.pool().clone(),
            ExecutorConfig::from_settings(&settings),
            registry,
            Arc::new(tokio::sync::RwLock::new(store)),
        ),
        ExecutionScope {
            job_id: creation.job.id.clone(),
            gate: Arc::clone(&gate),
            safe_retries: limits,
        },
    );
    let mut stop = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1800);
    started.store(true, std::sync::atomic::Ordering::SeqCst);
    loop {
        if let Err(code) = gate.check().await {
            stop = Some(code);
            break;
        }
        let mut conn = database
            .pool()
            .acquire()
            .await
            .map_err(|_| LiveError::new("storage"))?;
        let job = repo::jobs::get(&mut conn, &creation.job.id)
            .await
            .map_err(|_| LiveError::new("storage"))?
            .ok_or_else(|| LiveError::new("journal"))?;
        drop(conn);
        if matches!(
            job.status,
            manual_core::domain::JobStatus::Succeeded
                | manual_core::domain::JobStatus::Failed
                | manual_core::domain::JobStatus::NeedsInput
                | manual_core::domain::JobStatus::SubmissionUnknown
                | manual_core::domain::JobStatus::Cancelled
        ) {
            break;
        }
        tokio::select! {
            _=tokio::signal::ctrl_c()=> {stop=Some("interrupted");break;},
            result=executor.tick()=>match result {
                Ok(crate::jobs::TickOutcome::Idle)=>tokio::time::sleep(Duration::from_millis(100)).await,
                Ok(crate::jobs::TickOutcome::Executed(stage))=>println!("阶段 {} · {}",stage.stage_kind.as_str(),stage.outcome.code()),
                Err(crate::jobs::JobError::Authorization{code})=>{stop=Some(code);break;},
                Err(_)=>{stop=Some("executionReadFailed");break;},
            }
        }
        if tokio::time::Instant::now() >= deadline {
            stop = Some("resumeRequired");
            break;
        }
    }
    executor.request_shutdown();
    report::finish(
        report::ReportContext {
            case: &case,
            budget: &budget,
            plan: &journal.plan,
            pool: database.pool(),
            data_dir: &settings.data_dir,
            job_id: &creation.job.id,
            output: &output,
        },
        stop,
    )
    .await
}
