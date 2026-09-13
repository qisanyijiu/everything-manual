//! 子命令实现：init / serve / check / backup / restore（T02）。
//!
//! 输出约定：
//! - 面向人的结果行写 stdout（`check` 的有效配置摘要、`listening on ...`、结果行）；
//! - 结构化日志（JSON Lines）同时写 stdout 与 `<data-dir>/logs/everything-manual.log`；
//! - 错误由 `main.rs` 统一写 stderr（`错误：<消息>`），只带退出码，不带堆栈。

use std::ffi::OsString;
use std::net::SocketAddr;

use axum::Router;
use clap::Parser;

use super::cli::{
    BackupArgs, CheckArgs, Cli, Command, CommonArgs, InitArgs, RestoreArgs, ServeArgs,
};
use super::{CliError, CliOverrides, ExitCode, SecurityMode, Settings, datadir, logging, password};
use crate::storage::{Database, DatabaseStatus, StorageError};

/// 解析命令行并执行。返回 [`ExitCode`] 由 `main.rs` 转成进程退出码。
pub async fn run_from_args<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            if error.exit_code() == 0 {
                // --help / --version：正常输出到 stdout，退出 0。
                print!("{error}");
                return Ok(());
            }
            return Err(CliError::usage(error.to_string().trim_end().to_owned()));
        }
    };

    match cli.command {
        Command::Init(args) => run_init(&args).await,
        Command::Serve(args) => run_serve(&args).await,
        Command::Check(args) => run_check(&args).await,
        Command::Backup(args) => run_backup(&args).await,
        Command::Restore(args) => run_restore(&args).await,
    }
}

/// 数据库/schema 错误按 data-dir 错误（退出码 4）上报：`check`/`serve`/`init` 的
/// 数据库问题都属于"这个 data-dir 当前不可用"，与配置错误（3）区分（T02 退出码约定）。
fn storage_error(error: StorageError) -> CliError {
    CliError::data_dir(error.to_string())
}

fn overrides_from(common: &CommonArgs, listen: Option<SocketAddr>) -> CliOverrides {
    CliOverrides {
        data_dir: common.data_dir.clone(),
        config: common.config.clone(),
        listen,
    }
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/// `init --data-dir <dir> [--password-file <file>]`
///
/// 1. 解析配置（未知键报错）；
/// 2. 读取并校验管理员密码（交互终端不回显，或受限文件 0600；不存在命令行参数形式）；
/// 3. 创建 data-dir 结构（幂等）并获取排他锁（防止与 serve 并发初始化）；
/// 4. 创建/迁移数据库（schema 随二进制内嵌，旧库自动升级，新库拒绝打开）；
/// 5. **写入管理员凭据**：Argon2id 哈希进 `admins` 表（无默认密码）；已有管理员时
///    更新哈希并**撤销其全部既有会话**（重置密码路径）。明文密码不落盘、不进日志。
pub async fn run_init(args: &InitArgs) -> Result<(), CliError> {
    let settings = Settings::load(&overrides_from(&args.common, None))?;
    let secret = password::read_password(args.password_file.as_deref())?;
    let password_source = match args.password_file.as_deref() {
        Some(path) => format!("受限文件 {}", path.display()),
        None => "交互终端（不回显）".to_owned(),
    };

    let created = datadir::ensure_initialized(&settings.data_dir)?;
    let lock = datadir::DirLock::acquire(&settings.data_dir)?;
    logging::init(Some(&settings.data_dir.join("logs")))?;

    let database_existed = crate::storage::database_path(&settings.data_dir).is_file();
    let (database, migration) = Database::open_and_migrate_reporting(&settings.data_dir)
        .await
        .map_err(storage_error)?;
    let schema_version = database
        .applied_schema_version()
        .await
        .map_err(storage_error)?;

    // Argon2id（参数见 http::auth::password）；CPU/内存密集 → 阻塞线程池。
    let password_text = secret.expose().to_owned();
    let password_hash = tokio::task::spawn_blocking(move || {
        crate::http::auth::password::hash_password(&password_text)
    })
    .await
    .map_err(|error| CliError::internal(format!("口令哈希任务异常：{error}")))?
    .map_err(|error| CliError::internal(format!("口令哈希失败：{error}")))?;

    let mut connection = database
        .pool()
        .acquire()
        .await
        .map_err(|error| CliError::data_dir(format!("无法获取数据库连接：{error}")))?;
    let existing = crate::storage::repo::admins::get_single(&mut connection)
        .await
        .map_err(storage_error)?;
    let (admin_id, admin_status) = match existing {
        None => {
            let admin = crate::storage::repo::admins::insert(&mut connection, &password_hash)
                .await
                .map_err(storage_error)?;
            (admin.id, "已创建")
        }
        Some(admin) => {
            crate::storage::repo::admins::update_password(
                &mut connection,
                &admin.id,
                &password_hash,
            )
            .await
            .map_err(storage_error)?;
            let revoked = crate::storage::repo::sessions::revoke_all_for_admin(
                &mut connection,
                &admin.id,
                manual_core::timestamps::Timestamp::now(),
            )
            .await
            .map_err(storage_error)?;
            tracing::info!(
                event = "admin_password_reset",
                adminId = %admin.id,
                revokedSessions = revoked,
                "管理员密码已更新，既有会话已撤销"
            );
            (admin.id, "已更新")
        }
    };
    drop(connection);

    tracing::info!(
        event = "init",
        dataDir = %settings.data_dir.display(),
        passwordSource = %password_source,
        schemaVersion = schema_version,
        adminId = %admin_id,
        adminStatus = %admin_status,
        "data-dir 初始化完成"
    );

    println!("data-dir 已初始化：{}", settings.data_dir.display());
    for item in created {
        println!("  - {item}");
    }
    println!(
        "数据库：{}（{}，schema v{}；迁移随二进制内嵌，WAL / synchronous=FULL / foreign_keys=ON）",
        if database_existed {
            "已就绪"
        } else {
            "已创建并迁移"
        },
        crate::storage::database_path(&settings.data_dir).display(),
        schema_version
    );
    println!(
        "管理员凭据：{admin_status}（admins 表；{}；无默认密码）。",
        crate::http::auth::password::PARAMETERS_SUMMARY
    );
    if admin_status == "已更新" {
        println!("  既有会话已全部撤销（重置密码后需重新登录）。");
    }
    if migration.upgraded {
        println!(
            "  提示：数据库 schema 已自动升级 v{} → v{}（升级前应先备份；程序回滚不等于数据库回滚）。",
            migration.from, migration.to
        );
    }
    println!("密码来源：{password_source}；明文不落盘、不进入日志与 shell history。");

    database.close().await;
    drop(lock);
    Ok(())
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

/// `serve --data-dir <dir> [--listen <addr>]`
///
/// 启动顺序：配置解析 → 监听安全评估（非 loopback 需 TLS 或可信代理）→ **data-dir 结构校验**
/// （要求已 `init`，与 `check` 同一套校验，不隐式创建结构）→ 排他锁 → **打开数据库并自动迁移
/// schema**（库比程序新则拒绝打开，退出码 4）→ 绑定监听 → 服务。
/// 锁在进程退出（含被 Kill）时由操作系统释放。
pub async fn run_serve(args: &ServeArgs) -> Result<(), CliError> {
    let settings = Settings::load(&overrides_from(&args.common, args.listen))?;
    let mode = settings.evaluate_listen_security()?;
    if mode == SecurityMode::TrustedProxy {
        // 不默认相信任意 X-Forwarded-*：T04 起按 trusted_proxy_cidrs 判定来源后才使用转发头。
        tracing::warn!(
            event = "trusted_proxy_mode",
            listen = %settings.listen,
            trustedProxyCidrs = %settings
                .trusted_proxy_cidrs
                .iter()
                .map(super::Cidr::to_string)
                .collect::<Vec<_>>()
                .join(","),
            "反向代理模式：仅当反向代理来源位于 trusted_proxy_cidrs 时才可信任转发头（T04 起实现）"
        );
    }

    // 与 check 使用同一套结构校验：serve 不隐式创建结构，未初始化的目录明确要求先 init。
    datadir::verify(&settings.data_dir)?;
    logging::init(Some(&settings.data_dir.join("logs")))?;
    let lock = datadir::DirLock::acquire(&settings.data_dir)?;

    // 数据库：自动检测并迁移到程序支持版本；库比程序新 → 拒绝打开（不修改数据）。
    let (database, migration) = Database::open_and_migrate_reporting(&settings.data_dir)
        .await
        .map_err(storage_error)?;
    let schema_version = database
        .applied_schema_version()
        .await
        .map_err(storage_error)?;
    tracing::info!(
        event = "database_ready",
        dataDir = %settings.data_dir.display(),
        schemaVersion = schema_version,
        "数据库 schema 已就绪"
    );
    if migration.upgraded {
        // 升级审计 + 运维提示（T20）：升级前应先备份；回滚程序不等于回滚数据库。
        tracing::warn!(
            event = "schema_upgraded",
            from = migration.from,
            to = migration.to,
            "数据库 schema 已自动升级：升级前应先备份；回滚程序不能读取比它新的 schema"
        );
        println!(
            "提示：数据库 schema 已自动升级 v{} → v{}。升级前应先备份\
             （backup --data-dir <dir> --out <新路径>，需停服）；程序回滚不等于数据库回滚\
             （旧程序不能打开更新的 schema，只能用迁移前备份恢复到新目录）。",
            migration.from, migration.to
        );
    }

    // 崩溃残留按引用扫描（T06）：启动时没有在途上传，tmp 里的文件必然是崩溃残留 →
    // 隔离到 quarantine/；未被任何 blob 行引用的孤儿文件同样隔离（只移动不删除）。
    // 扫描失败不阻塞启动（此时还没有任何资产流量），只留下明确日志。
    match database.pool().acquire().await {
        Ok(mut connection) => {
            match crate::assets::maintenance::scan_and_quarantine(
                &mut connection,
                &settings.data_dir,
            )
            .await
            {
                Ok(report) => {
                    tracing::info!(
                        event = "asset_scan",
                        tmpQuarantined = report.tmp_quarantined,
                        blobsQuarantined = report.blobs_quarantined,
                        blobsKept = report.blobs_kept,
                        blobsRestored = report.blobs_restored,
                        blobsMarkedMissing = report.blobs_marked_missing,
                        "资产残留扫描完成（隔离只移动不删除）"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error.log_summary(),
                        "资产残留扫描失败（不阻塞启动）"
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "资产残留扫描：获取数据库连接失败（跳过本次扫描）");
        }
    }

    // 持久任务执行器（T10）：与 HTTP 同进程，SQLite 是事实来源。
    // 阶段处理器按 Provider 配置注册（T12 起：Tripo 的 upload/submit/poll；
    // T13/T14/T15 补下载/校验/说明书 AI/组装）。已配置但初始化失败 = 配置错误，
    // 直接拒绝启动（不静默降级为"未配置"）；未注册的阶段会被延后并记录，**不假成功**。
    let executor_config = crate::jobs::ExecutorConfig::from_settings(&settings);
    executor_config
        .validate()
        .map_err(|error| CliError::config(error.to_string()))?;
    let mut registry = crate::jobs::StageRegistry::new();
    crate::providers::register_provider_handlers(&mut registry, &settings)
        .map_err(|error| CliError::config(error.to_string()))?;
    // T15：组装草稿是本地阶段（零外呼、零新费用），不依赖任何 Provider 配置——
    // 无条件注册，否则"两条分支都跑完却永远组装不出草稿"（未注册阶段只会被延后）。
    let pipeline = crate::jobs::PipelineHandlers::from_settings(&settings);
    let pipeline_stages = pipeline.register(&mut registry);
    tracing::info!(
        event = "pipeline_handlers_registered",
        stages = %pipeline_stages
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(","),
        "组装阶段处理器已注册（本地确定性组装；不存在自动发布）"
    );
    if registry.registered().len() == pipeline_stages.len() {
        tracing::warn!(
            event = "job_executor_no_handlers",
            "任务执行器未注册任何 Provider 阶段处理器（Provider 未配置）：付费阶段会被延后；\
             组装阶段不依赖 Provider，仍正常注册"
        );
    }
    let executor =
        crate::jobs::JobExecutor::new(database.pool().clone(), executor_config.clone(), registry);
    tracing::info!(
        event = "job_executor_start",
        owner = executor.owner(),
        leaseSeconds = executor_config.lease.as_secs(),
        renewSeconds = executor_config.renew.as_secs(),
        remoteGenerationLimit = executor_config.remote_generation_limit,
        manualAiBatchLimit = executor_config.manual_ai_batch_limit,
        "任务执行器启动（SQLite 事实来源；崩溃断点仅测试构建）"
    );
    let executor_handle = executor.start();

    let listener = tokio::net::TcpListener::bind(settings.listen)
        .await
        .map_err(|error| {
            CliError::internal(format!(
                "无法监听 {}：{error}（端口可能已被占用）",
                settings.listen
            ))
        })?;
    let addr = listener
        .local_addr()
        .map_err(|error| CliError::internal(format!("读取监听地址失败：{error}")))?;

    for provider in [&settings.providers.tripo, &settings.providers.manual_ai] {
        if provider.configured() {
            tracing::info!(
                event = "provider_configured",
                provider = provider.name,
                baseUrl = %super::secret::redact_url_query(&provider.base_url),
                keySource = provider.key_source.as_deref().unwrap_or("未知"),
                "Provider 已配置"
            );
        } else {
            tracing::warn!(
                event = "provider_not_configured",
                provider = provider.name,
                missing = %provider.missing().join(","),
                "Provider 未配置：生成与报价能力将返回“未配置”，不回退 mock；已有资料仍可读"
            );
        }
    }

    tracing::info!(
        event = "serve_start",
        listen = %addr,
        dataDir = %settings.data_dir.display(),
        securityMode = mode.as_str(),
        sessionTtlHours = settings.session.ttl_hours,
        loginRateLimitPerMinute = settings.session.login_rate_limit_per_minute,
        cookieSecure = settings.cookie_secure(),
        "服务启动"
    );
    // 该行是 cargo xtask smoke-bootstrap 的启动协议，保持格式不变。
    println!("listening on http://{addr}");

    // 应用状态：数据库（clone 共享连接池）+ 配置（限速/TTL/cookie Secure 判定）。
    let state = crate::http::state::AppState::new(database.clone(), settings.clone());

    axum::serve(
        listener,
        build_app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|error| CliError::internal(format!("HTTP 服务异常退出：{error}")))?;

    // 退出顺序：先停 HTTP（上面已返回）→ 任务执行器停止领取并等待在途短写入/checkpoint
    // → 关闭数据库连接池 → 释放 data-dir 锁。
    executor_handle.shutdown().await;
    database.close().await;
    tracing::info!(event = "serve_stop", "收到终止信号，服务已停止");
    drop(lock);
    println!("已停止：data-dir 排他锁已释放。");
    Ok(())
}

/// SIGINT / SIGTERM 触发优雅退出（停止领取后续任务由 T10 接入）。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

/// `check --data-dir <dir>`
///
/// 只做本地校验：配置解析、data-dir 结构、可写性、排他锁可用性、**数据库 schema
/// （可打开、迁移状态、版本兼容）**与监听安全。
/// **不发起任何外部 HTTP 请求、不产生费用**（AC-006）；也不修改 data-dir 内容
/// （唯一例外是被删除的写探针与追加的日志）。
///
/// 数据库检查是**只读**的（[`crate::storage::db::inspect`]）：不创建数据库、不应用迁移；
/// 旧 schema 只报告"待迁移"，实际迁移发生在 `init`/`serve`。库比程序新 → 退出码 4。
pub async fn run_check(args: &CheckArgs) -> Result<(), CliError> {
    let settings = Settings::load(&overrides_from(&args.common, args.listen))?;

    println!("有效配置（脱敏）：");
    for line in settings.summary_lines() {
        println!("  {line}");
    }

    let ok_items = datadir::verify(&settings.data_dir)?;
    println!(
        "[检查] data-dir 结构与锁文件：通过（{}）",
        ok_items.join("、")
    );
    datadir::probe_writable(&settings.data_dir)?;
    println!("[检查] data-dir 可写：通过");

    // 锁：空闲则通过；被占用说明有服务在运行（合法状态），只警告不失败。
    match datadir::DirLock::acquire(&settings.data_dir) {
        Ok(lock) => {
            drop(lock);
            println!("[检查] 排他锁：通过（当前空闲，可启动 serve）");
        }
        Err(error) if error.exit_code == ExitCode::Locked => {
            println!("[检查] 排他锁：警告——{}", error.message);
        }
        Err(error) => return Err(error),
    }

    // 数据库 schema：只读检查（不创建、不迁移）。库比程序新 → 明确拒绝（退出码 4）。
    match crate::storage::db::inspect(&settings.data_dir)
        .await
        .map_err(storage_error)?
    {
        DatabaseStatus::Missing { program_version } => println!(
            "[检查] 数据库 schema：尚未建立（init 或首次 serve 时自动创建并迁移到 v{program_version}）"
        ),
        DatabaseStatus::Ready {
            version,
            settings: connection,
        } => println!(
            "[检查] 数据库 schema：已就绪（v{version}；{}）",
            connection.summary()
        ),
        DatabaseStatus::Pending { applied, program } => {
            println!(
                "[检查] 数据库 schema：待迁移（库 v{applied} → 程序 v{program}；\
                 serve/init 启动时自动迁移，本命令不修改数据）"
            );
            // 升级前备份提示（T20；validation-release §5「备份和升级」第 1、3、4 步）。
            println!(
                "[检查] 升级提示：迁移前请先备份——停止服务后执行 \
                 `backup --data-dir <本目录> --out <不存在的新路径>`；\
                 程序回滚不等于数据库回滚（旧程序不能打开更新的 schema，只能用迁移前备份恢复到新目录）"
            );
        }
    }

    let mode = settings.evaluate_listen_security()?;
    println!("[检查] 监听安全：通过（{}）", mode.as_str());

    for provider in [&settings.providers.tripo, &settings.providers.manual_ai] {
        println!(
            "[检查] Provider {}：{}",
            provider.name,
            provider.status_line()
        );
    }

    // 日志初始化失败不影响检查结论（例如只读日志目录会被前面可写性检查先拦下）。
    match logging::init(Some(&settings.data_dir.join("logs"))) {
        Ok(()) => tracing::info!(event = "check", "check 完成（未发起任何外部请求）"),
        Err(error) => println!("[检查] 日志文件：警告——{}", error.message),
    }
    println!("[检查] 外部请求：本命令不发起任何网络请求（0 次）");
    println!("结果：check 通过");
    Ok(())
}

// ---------------------------------------------------------------------------
// backup / restore（T20 实现；validation-release.md §5）
// ---------------------------------------------------------------------------

/// 备份／恢复错误的退出码映射（合同见 `crate::backup` 模块文档与 T20 退出码表）：
///
/// | 错误分类 | 退出码 |
/// | --- | --- |
/// | 路径/前置条件（目标非空、输出已存在、库比程序新） | 4 |
/// | 排他锁冲突（backup 要求先停服） | 5 |
/// | 完整性校验失败（hash 不符、blob 损坏/缺失、外键与引用坏） | 7 |
/// | 运行时 I/O | 1 |
/// | 存储层 | 4（与 check/serve 的 data-dir 错误一致） |
fn backup_error(error: crate::backup::BackupError) -> CliError {
    use crate::backup::BackupError;
    match error {
        BackupError::Path { message } => CliError::data_dir(message),
        BackupError::Locked { message } => CliError::locked(message),
        BackupError::Integrity { message, .. } => CliError::integrity(message),
        BackupError::Io { message } => CliError::internal(message),
        BackupError::Storage(error) => storage_error(error),
    }
}

/// `backup --data-dir <dir> --out <不存在的新路径>`
///
/// 步骤（validation-release §5「备份和升级」1–2）：
/// 1. 解析配置与 `--out`（相对路径相对进程工作目录）；
/// 2. 校验 data-dir 结构并**获取排他锁**——服务在运行时这一步失败（退出码 5），
///    错误明确要求先停服（AC-009："运行中请求被拒绝并说明需先停服"）；
/// 3. 生成一致 SQLite 快照（`VACUUM INTO`，含 WAL 中已提交事务；不是复制主文件）
///    + 全部被引用 blob + manifest + sha256；快照内清空会话；
/// 4. `--out` 必须不存在：绝不覆盖已有备份或文件。
///
/// 升级前备份：本命令就是升级流程的第 1 步；程序回滚不等于数据库回滚
/// （旧程序不能打开比它新的 schema，见 `serve`/`check` 的升级提示）。
pub async fn run_backup(args: &BackupArgs) -> Result<(), CliError> {
    let settings = Settings::load(&overrides_from(&args.common, None))?;
    let out = absolute_path(&args.out)?;

    // 快速失败：输出路径已存在时不做任何其它动作（不获取锁、不建目录）。
    if out.exists() {
        return Err(CliError::data_dir(format!(
            "备份输出路径已存在，绝不覆盖：{}；请换一个不存在的路径",
            out.display()
        )));
    }
    datadir::verify(&settings.data_dir)?;
    let lock = datadir::DirLock::acquire(&settings.data_dir).map_err(|error| {
        if error.exit_code == ExitCode::Locked {
            CliError::locked(format!(
                "备份要求先停止服务（data-dir 排他锁被占用）：{}。\
                 请停止服务后重新执行 backup（运行中备份无法保证一致快照）",
                error.message
            ))
        } else {
            error
        }
    })?;
    logging::init(Some(&settings.data_dir.join("logs")))?;

    let outcome = crate::backup::create_backup(&settings.data_dir, &out)
        .await
        .map_err(backup_error)?;

    println!("备份完成：{}", outcome.out_dir.display());
    println!(
        "  一致快照：database/manual.sqlite3（sha256 {}，{} 字节，schema v{}；VACUUM INTO，含 WAL 中已提交事务）",
        outcome.database_sha256, outcome.database_size, outcome.schema_version
    );
    println!(
        "  被引用 blob：复制 {} 个{}",
        outcome.blobs_copied,
        if outcome.missing_blobs > 0 {
            format!(
                "；**另有 {} 个 blob 在源 data-dir 中文件缺失**（manifest 的 missingBlobs 已如实记录）",
                outcome.missing_blobs
            )
        } else {
            String::new()
        }
    );
    println!(
        "  manifest 与 sha256：manifest.json；会话已从快照中清空（恢复后需重新登录，共清空 {} 条）",
        outcome.sessions_removed_count
    );
    println!(
        "  供应商临时 URL 脱敏：快照内 {} 处临时/签名地址已替换为 sha256 摘要 + host（AC-010；恢复后按 task_id 重新查询链接）",
        outcome.temp_urls_redacted
    );
    println!(
        "  恢复方式：everything-manual restore --from {} --data-dir <不存在或为空的新目录>",
        outcome.out_dir.display()
    );
    drop(lock);
    Ok(())
}

/// `restore --from <备份目录> --data-dir <不存在或为空的新目录>`
///
/// 步骤（validation-release §5「备份和升级」5）：目标必须不存在或为空；先全量校验
/// （manifest、快照与所有 blob 的 sha256、schema 版本、外键与引用），通过后才写入；
/// 任何校验失败都不创建/修改目标目录，并保留备份现场（退出码 7）。
pub async fn run_restore(args: &RestoreArgs) -> Result<(), CliError> {
    let settings = Settings::load(&overrides_from(&args.common, None))?;
    let from = absolute_path(&args.from)?;
    let target = settings.data_dir.clone();

    let outcome = crate::backup::restore_backup(&from, &target)
        .await
        .map_err(backup_error)?;

    // 恢复完成后才初始化日志（目标目录此时才存在；失败路径不写目标）。
    let _ = logging::init(Some(&target.join("logs")));
    tracing::info!(
        event = "restore_cli_completed",
        from = %from.display(),
        dataDir = %target.display(),
        schemaVersion = outcome.schema_version,
        "恢复完成"
    );

    println!("恢复完成：{}", outcome.data_dir.display());
    println!(
        "  来源备份：{}（schema v{}；快照 sha256 {}）",
        outcome.backup_dir.display(),
        outcome.schema_version,
        outcome.database_sha256
    );
    println!(
        "  校验通过：manifest、快照与全部 blob 的 sha256、外键与引用；恢复 blob {} 个{}",
        outcome.blobs_restored,
        if outcome.missing_blobs > 0 {
            format!(
                "（源备份中有 {} 个 blob 本来就缺失，已如实保留状态）",
                outcome.missing_blobs
            )
        } else {
            String::new()
        }
    );
    println!(
        "  数据规模：items {}、assets {}、releases {}、blobs {}",
        outcome.counts.items, outcome.counts.assets, outcome.counts.releases, outcome.counts.blobs
    );
    println!(
        "  下一步：everything-manual serve --data-dir {}（schema 旧于程序时启动会自动迁移；\
         升级前请再次备份，程序回滚不等于数据库回滚）",
        outcome.data_dir.display()
    );
    Ok(())
}

/// 把 CLI 传入的路径解析为绝对路径（相对路径相对**进程工作目录**，与 T02 的
/// 配置路径规则一致；避免"归档/运行位置不同导致解析不同"）。
fn absolute_path(path: &std::path::Path) -> Result<std::path::PathBuf, CliError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir()
        .map_err(|error| CliError::internal(format!("无法获取当前工作目录：{error}")))?;
    Ok(cwd.join(path))
}

fn build_app(state: crate::http::state::AppState) -> Router {
    crate::http::router::build_app(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn help_exits_zero_without_error() {
        run_from_args(["everything-manual", "--help"])
            .await
            .unwrap();
        run_from_args(["everything-manual", "--version"])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unknown_subcommand_maps_to_usage_error() {
        let error = run_from_args(["everything-manual", "frobnicate"])
            .await
            .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Usage);
        assert!(error.message.contains("frobnicate"), "{}", error.message);
    }

    /// T20：backup/restore 已实现；这里只测**不触碰任何目录**的错误路径退出码。
    /// 正常/更多错误路径在集成测试 `backup_restore.rs`（真实 CLI 子进程）。
    #[tokio::test]
    async fn backup_and_restore_fail_with_documented_codes_without_side_effects() {
        let base = std::env::temp_dir().join(format!(
            "em-commands-unit-{}-{}",
            std::process::id(),
            manual_core::timestamps::Timestamp::now().as_millis()
        ));
        let missing_data = base.join("missing-data");
        let out = base.join("backup-out");

        // 源 data-dir 不存在 → 4（data-dir 错误），且不创建输出。
        let backup = run_from_args([
            "everything-manual",
            "backup",
            "--data-dir",
            missing_data.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .await
        .unwrap_err();
        assert_eq!(backup.exit_code, ExitCode::DataDir, "{}", backup.message);
        assert!(!out.exists(), "失败路径不得创建输出目录");

        // 备份来源不存在 → 4，且不创建目标。
        let target = base.join("restored");
        let restore = run_from_args([
            "everything-manual",
            "restore",
            "--from",
            base.join("no-backup").to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ])
        .await
        .unwrap_err();
        assert_eq!(restore.exit_code, ExitCode::DataDir, "{}", restore.message);
        assert!(!target.exists(), "失败路径不得创建目标目录");
    }

    #[test]
    fn backup_error_mapping_is_stable() {
        use crate::backup::BackupError;
        assert_eq!(
            backup_error(BackupError::path("x")).exit_code,
            ExitCode::DataDir
        );
        assert_eq!(
            backup_error(BackupError::locked("x")).exit_code,
            ExitCode::Locked
        );
        assert_eq!(
            backup_error(BackupError::integrity("backup_blob_corrupt", "x")).exit_code,
            ExitCode::Integrity
        );
        assert_eq!(
            backup_error(BackupError::io("x")).exit_code,
            ExitCode::Internal
        );
    }
}
