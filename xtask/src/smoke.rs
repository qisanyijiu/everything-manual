//! `cargo xtask smoke --binary <绝对路径>` —— T22 正式包冷目录冒烟
//! （validation-release.md §7 的 7 步；命令合同见 §2）。
//!
//! 与 `smoke-bootstrap`（T01 最小检查）的区别：本命令验证**正式发布包**——
//! 从 T20 的合法脱敏样例备份恢复到新空 data-dir，用生产 `embedded-ui` 构图完成
//! 认证、静态资源、资产读取（Range/HEAD）、持久化、断外网读取与「备份→恢复→再读」
//! 闭环；**不启动本机 Provider fixture、不放行任何 fixture URL**（子进程环境清空为
//! `PATH=/usr/bin:/bin`，工作目录不在仓库内），未配置 Provider 时生成必须明确拒绝。
//!
//! 步骤编号与 §7 一致：
//! 1. 新临时目录只放 binary（另建独立临时 data-dir，不复用仓库）；
//! 2. `restore` 样例备份到新空目录并启动生产服务；另用独立空目录验证 `init`；
//! 3. 登录后验证首页、嵌套路由刷新、JS/CSS/字体/PDF 资源、未知 `/api/*` JSON 404、
//!    未知带扩展名静态资源不得返回 HTML 200；
//! 4. 读取恢复出的 release／模型／PDF：Range/HEAD/ETag；未配置 Provider 时报价被
//!    明确拒绝（409 `PROVIDER_NOT_CONFIGURED`）；
//! 5. 断外网仍可读取已有 release／PDF／GLB（macOS 用 sandbox-exec 拒绝非 localhost
//!    出站；Linux 容器在 `--network none` 下运行整条命令）；
//! 6. 停服重启后数据仍在（manifest 与资产 sha 比对）；
//! 7. `backup` → 新目录 `restore` → 再读同一 release，比对 manifest/hash。
//!
//! 只结束自己启动的进程（[`Service`] 的 Drop 保证），失败时可用 `--keep` 保留现场。

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, Response};

use crate::util::{repo_root, sha256_bytes};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// T20 手工演练用的合法脱敏样例备份（`artifacts/web-mvp/t20-rd/sample-backup/`）。
const DEFAULT_BACKUP: &str = "artifacts/web-mvp/t20-rd/sample-backup";
/// 样例备份的测试管理员口令（非生产凭据；来自 T20 演练记录）。
const DEFAULT_PASSWORD: &str = "test-password-t20-backup";
const BINARY_NAME: &str = "everything-manual";
/// 未配置 Provider 时报价必须返回的错误码（generation::GenerationError::ProviderNotConfigured）。
const NOT_CONFIGURED_CODE: &str = "PROVIDER_NOT_CONFIGURED";
/// 沙箱 profile：默认放行，但拒绝一切非 localhost 出站连接（＝断外网，保留回环本地读取）。
const OFFLINE_SANDBOX_PROFILE: &str = "(version 1)\n(allow default)\n(deny network-outbound)\n\
     (allow network-outbound (remote ip \"localhost:*\"))\n";

type LogBuffer = Arc<Mutex<Vec<String>>>;

// ---------------------------------------------------------------------------
// T01 smoke-bootstrap（保持原语义）
// ---------------------------------------------------------------------------

pub fn run_bootstrap(binary: &Path) -> Result<()> {
    if !binary.is_absolute() {
        bail!("--binary 必须是绝对路径：{}", binary.display());
    }
    if !binary.is_file() {
        bail!("--binary 不存在或不是文件：{}", binary.display());
    }

    let work_dir = temp_work_dir("smoke-bootstrap");
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("创建临时目录失败：{}", work_dir.display()))?;
    let local_binary = work_dir.join(BINARY_NAME);
    copy_binary(binary, &local_binary)?;

    println!(
        "冒烟目录：{}（仅含二进制与其 data-dir，工作目录即该目录）",
        work_dir.display()
    );

    let data_dir = work_dir.join("data");
    let password_file = work_dir.join("init-password");
    write_password_file(&password_file, "smoke-bootstrap-password")?;

    let init = Command::new(&local_binary)
        .arg("init")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--password-file")
        .arg(&password_file)
        .current_dir(&work_dir)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("无法执行 init：{}", local_binary.display()))?;
    if !init.status.success() {
        bail!(
            "init 失败（退出码 {:?}）：\nstdout: {}\nstderr: {}",
            init.status.code(),
            String::from_utf8_lossy(&init.stdout),
            String::from_utf8_lossy(&init.stderr)
        );
    }
    println!("  [准备] init --data-dir data 成功");

    let log: LogBuffer = Arc::new(Mutex::new(Vec::new()));
    let mut service = Service::start(&local_binary, &work_dir, &data_dir, Some(&log))?;
    let base_url = service.base_url.clone();
    println!("服务地址：{base_url}");
    let result = run_bootstrap_http_checks(&base_url);
    service.stop();
    if result.is_err() {
        print_log_tail(&log, 40);
    }

    let cleanup = std::fs::remove_dir_all(&work_dir);

    result?;
    cleanup.with_context(|| format!("清理临时目录失败：{}", work_dir.display()))?;
    println!("\n冒烟通过：内嵌页面、静态资源与 health 均可用（T01 最小检查）。");
    Ok(())
}

fn run_bootstrap_http_checks(base_url: &str) -> Result<()> {
    let client = http_client()?;

    let root = client.get(base_url).send().context("请求 / 失败")?;
    let root_status = root.status().as_u16();
    let root_type = content_type(&root);
    let root_body = root.text().context("读取 / 响应体失败")?;
    println!("  [检查] GET / -> {root_status} {root_type}");
    if root_status != 200 || !root_type.starts_with("text/html") {
        bail!("根路径应返回 200 text/html，实际 {root_status} {root_type}");
    }
    if !root_body.contains("id=\"root\"") {
        bail!("内嵌 index.html 缺少挂载点 id=\"root\"");
    }

    let asset_path = root_body
        .split('"')
        .find(|part| {
            part.starts_with("/assets/") && (part.ends_with(".js") || part.ends_with(".mjs"))
        })
        .map(str::to_owned)
        .context("内嵌 index.html 未引用 /assets/*.js")?;
    let asset = client
        .get(format!("{base_url}{asset_path}"))
        .send()
        .with_context(|| format!("请求 {asset_path} 失败"))?;
    let asset_status = asset.status().as_u16();
    let asset_type = content_type(&asset);
    let asset_body = asset.text().unwrap_or_default();
    println!("  [检查] GET {asset_path} -> {asset_status} {asset_type}");
    if asset_status != 200 || !asset_type.contains("javascript") || asset_body.is_empty() {
        bail!(
            "内嵌 JS 静态资源异常：{asset_status} {asset_type}（长度 {}）",
            asset_body.len()
        );
    }

    let live = client
        .get(format!("{base_url}/api/v1/health/live"))
        .send()
        .context("请求 health/live 失败")?;
    let live_status = live.status().as_u16();
    let live_body: serde_json::Value = live.json().context("health/live 响应不是 JSON")?;
    println!("  [检查] GET /api/v1/health/live -> {live_status}");
    if live_status != 200 || live_body["data"]["status"] != "ok" {
        bail!("health/live 响应异常：{live_status} {live_body}");
    }

    let ready = client
        .get(format!("{base_url}/api/v1/health/ready"))
        .send()
        .context("请求 health/ready 失败")?;
    let ready_status = ready.status().as_u16();
    let ready_body: serde_json::Value = ready.json().context("health/ready 响应不是 JSON")?;
    println!("  [检查] GET /api/v1/health/ready -> {ready_status}");
    if ready_status != 200 || ready_body["data"]["status"] != "ready" {
        bail!("health/ready 响应异常：{ready_status} {ready_body}");
    }

    let unknown = client
        .get(format!("{base_url}/api/unknown"))
        .send()
        .context("请求 /api/unknown 失败")?;
    let unknown_status = unknown.status().as_u16();
    let unknown_type = content_type(&unknown);
    let unknown_body: serde_json::Value = unknown.json().context("未知 API 必须返回 JSON 404")?;
    println!("  [检查] GET /api/unknown -> {unknown_status} {unknown_type}");
    if unknown_status != 404 || !unknown_type.starts_with("application/json") {
        bail!("未知 /api/* 应返回 JSON 404，实际 {unknown_status} {unknown_type}");
    }
    if unknown_body["error"]["code"] != "NOT_FOUND" {
        bail!("未知 /api/* 错误码应为 NOT_FOUND，实际 {unknown_body}");
    }

    let deep_link = client
        .get(format!("{base_url}/library/some-item"))
        .send()
        .context("请求 SPA 深链接失败")?;
    let deep_status = deep_link.status().as_u16();
    let deep_type = content_type(&deep_link);
    println!("  [检查] GET /library/some-item -> {deep_status} {deep_type}");
    if deep_status != 200 || !deep_type.starts_with("text/html") {
        bail!("SPA 深链接应返回 index.html，实际 {deep_status} {deep_type}");
    }

    let missing = client
        .get(format!("{base_url}/assets/definitely-missing.js"))
        .send()
        .context("请求缺失静态资源失败")?;
    let missing_status = missing.status().as_u16();
    let missing_type = content_type(&missing);
    println!("  [检查] GET /assets/definitely-missing.js -> {missing_status} {missing_type}");
    if missing_status != 404 || missing_type.starts_with("text/html") {
        bail!("缺失静态资源应 404 且不返回 HTML，实际 {missing_status} {missing_type}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// T22 正式包 smoke
// ---------------------------------------------------------------------------

/// `cargo xtask smoke` 的入参（CLI 见 main.rs）。
pub struct SmokeOptions {
    pub binary: PathBuf,
    pub backup: Option<PathBuf>,
    pub password: Option<String>,
    pub keep: bool,
    pub skip_offline_sandbox: bool,
}

pub fn run(options: &SmokeOptions) -> Result<()> {
    let repo = repo_root();
    let binary = &options.binary;
    if !binary.is_absolute() {
        bail!("--binary 必须是绝对路径：{}", binary.display());
    }
    if !binary.is_file() {
        bail!("--binary 不存在或不是文件：{}", binary.display());
    }

    let backup = match &options.backup {
        Some(path) => std::fs::canonicalize(path)
            .with_context(|| format!("--backup 路径无效：{}", path.display()))?,
        None => repo.join(DEFAULT_BACKUP),
    };
    if !backup.is_dir() {
        bail!(
            "样例备份目录不存在：{}（在仓库根执行，或用 --backup 指定 T20 合法备份）",
            backup.display()
        );
    }
    let password = options
        .password
        .clone()
        .unwrap_or_else(|| DEFAULT_PASSWORD.to_owned());

    let work_dir = temp_work_dir("smoke");
    if work_dir.starts_with(&repo) {
        bail!("拒绝在仓库内创建冒烟临时目录：{}", work_dir.display());
    }
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("创建临时目录失败：{}", work_dir.display()))?;

    let smoke = Smoke {
        work_dir: work_dir.clone(),
        binary: binary.clone(),
        backup: backup.clone(),
        password,
        keep: options.keep,
        skip_offline_sandbox: options.skip_offline_sandbox,
        log: Arc::new(Mutex::new(Vec::new())),
    };

    println!("=== T22 正式包冷目录冒烟（validation-release §7 的 7 步）===");
    println!("binary   : {}", binary.display());
    println!("backup   : {}", backup.display());
    println!("临时目录 : {}", work_dir.display());

    let result = smoke.run_steps();
    if result.is_err() {
        print_log_tail(&smoke.log, 40);
    }
    if smoke.keep {
        println!("\n保留临时目录（--keep）：{}", work_dir.display());
    } else {
        match std::fs::remove_dir_all(&work_dir) {
            Ok(()) => println!("临时目录已清理：{}", work_dir.display()),
            Err(error) => eprintln!("清理临时目录失败（{error}）：{}", work_dir.display()),
        }
    }

    result?;
    println!("\n正式包冒烟通过（7 步全过；步骤编号与 validation-release §7 一致）。");
    Ok(())
}

struct Smoke {
    work_dir: PathBuf,
    binary: PathBuf,
    backup: PathBuf,
    password: String,
    keep: bool,
    skip_offline_sandbox: bool,
    log: LogBuffer,
}

impl Smoke {
    fn run_steps(&self) -> Result<()> {
        let local_binary = self.work_dir.join(BINARY_NAME);
        let client = http_client()?;

        // ---- 步骤 1：新临时目录只放二进制 ----
        copy_binary(&self.binary, &local_binary)?;
        let entries: Vec<String> = std::fs::read_dir(&self.work_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        if entries.len() != 1 || entries[0] != BINARY_NAME {
            bail!("步骤 1 失败：冒烟目录应只含 {BINARY_NAME}，实际 {entries:?}");
        }
        println!(
            "[步骤 1] 冷目录已建立：{}（只含二进制；无源码 / dist / Node）",
            self.work_dir.display()
        );

        // ---- 步骤 2：restore 样例备份 + init 独立空目录 + 启动生产服务 ----
        let data_dir = self.work_dir.join("data");
        let restore_output = run_binary(
            &local_binary,
            &self.work_dir,
            &[
                "restore",
                "--from",
                &self.backup.display().to_string(),
                "--data-dir",
                &data_dir.display().to_string(),
            ],
        )?;
        println!(
            "[步骤 2] restore 样例备份 → {}（退出码 0）",
            data_dir.display()
        );
        for line in restore_output.lines().take(4) {
            println!("         {line}");
        }

        let init_dir = self.work_dir.join("init-data");
        let password_file = self.work_dir.join("init-password");
        write_password_file(&password_file, "smoke-t22-init-password")?;
        run_binary(
            &local_binary,
            &self.work_dir,
            &[
                "init",
                "--data-dir",
                &init_dir.display().to_string(),
                "--password-file",
                &password_file.display().to_string(),
            ],
        )?;
        run_binary(
            &local_binary,
            &self.work_dir,
            &["check", "--data-dir", &init_dir.display().to_string()],
        )?;
        println!(
            "[步骤 2] 独立空目录 init + check 通过：{}（新部署路径）",
            init_dir.display()
        );

        let mut service =
            Service::start(&local_binary, &self.work_dir, &data_dir, Some(&self.log))?;
        println!(
            "[步骤 2] 生产服务已启动：PID {} {}（环境清空，工作目录不在仓库内）",
            service.pid, service.base_url
        );
        check_no_child_processes(service.pid);

        // ---- 步骤 3：静态资源 / 嵌套路由 / 未知 API ----
        let index_html = fetch_index(&client, &service.base_url)?;
        check_static_surface(&client, &service.base_url, &index_html)?;

        // ---- 步骤 4：认证 + release/资产读取 + Range/HEAD + 未配置 Provider 拒绝 ----
        let unauthenticated = client
            .get(format!("{}/api/v1/items", service.base_url))
            .send()
            .context("请求未认证 items 失败")?;
        if unauthenticated.status().as_u16() != 401 {
            bail!(
                "未登录访问 /api/v1/items 应 401，实际 {}",
                unauthenticated.status()
            );
        }
        println!("  [检查] 未登录 GET /api/v1/items -> 401（认证真实生效）");

        let session = Session::login(&client, &service.base_url, &self.password)?;
        println!("  [检查] 登录成功（cookie HttpOnly + SameSite=Strict；CSRF token 已获取）");

        let settings = session.get_json("/api/v1/settings/status")?;
        let tripo = &settings["data"]["providers"]["tripo"]["providersConfigured"];
        let manual_ai = &settings["data"]["providers"]["manualAi"]["providersConfigured"];
        if tripo == true || manual_ai == true {
            bail!("正式包冒烟不得连接 Provider fixture：settings/status 显示已配置（{settings}）");
        }
        println!("  [检查] providersConfigured=false（不放行 fixture / 未连接本机 Provider）");

        let release = read_release(&session)?;
        let backup_index = BackupIndex::read(&self.backup)?;
        if !backup_index.blob_sha256.contains(&release.manifest_sha256) {
            bail!(
                "release manifest sha256 {} 不在备份 blob 清单中（恢复数据与备份不一致）",
                release.manifest_sha256
            );
        }
        if release.model_sha256 != release.model_declared_sha256
            || release.pdf_sha256 != release.pdf_declared_sha256
        {
            bail!(
                "manifest 声明的资产 sha 与实际 blob 不一致：model {} vs {}；pdf {} vs {}",
                release.model_sha256,
                release.model_declared_sha256,
                release.pdf_sha256,
                release.pdf_declared_sha256
            );
        }
        println!(
            "  [检查] release {} 的 manifestSha256={} 命中备份 blob 清单；模型 {} / PDF {} 与 manifest 声明一致",
            release.release_id,
            &release.manifest_sha256[..12],
            &release.model_sha256[..12],
            &release.pdf_sha256[..12]
        );

        check_asset(
            &session,
            &release.model_asset_id,
            &release.model_sha256,
            "GLB",
        )?;
        check_asset(&session, &release.pdf_asset_id, &release.pdf_sha256, "PDF")?;
        check_unconfigured_rejection(&session, &release.item_id, &release.preparation_id)?;

        let ready = session.get_json("/api/v1/health/ready")?;
        if ready["data"]["status"] != "ready" {
            bail!("ready 状态异常（Provider 未配置不应影响本地读取）：{ready}");
        }
        println!("  [检查] /health/ready = ready（Provider 未配置不影响本地读取）");

        let first_seen = SealedState {
            manifest_sha256: release.manifest_sha256.clone(),
            model_sha256: release.model_sha256.clone(),
            pdf_sha256: release.pdf_sha256.clone(),
        };
        drop(session);
        service.stop();

        // ---- 步骤 5：断外网读取（沙箱内重启服务） ----
        self.check_offline(&client, &local_binary, &data_dir, &first_seen)?;

        // ---- 步骤 6：停服重启后数据仍在 ----
        let mut service =
            Service::start(&local_binary, &self.work_dir, &data_dir, Some(&self.log))?;
        let session = Session::login(&client, &service.base_url, &self.password)?;
        let restarted = read_release(&session)?;
        if restarted.manifest_sha256 != first_seen.manifest_sha256
            || restarted.model_sha256 != first_seen.model_sha256
            || restarted.pdf_sha256 != first_seen.pdf_sha256
        {
            bail!("重启后数据发生变化：{restarted:?} != {first_seen:?}");
        }
        check_asset(
            &session,
            &restarted.model_asset_id,
            &restarted.model_sha256,
            "GLB(重启后)",
        )?;
        println!("[步骤 6] 停服重启后数据仍在：manifest/模型/PDF sha256 与首轮一致");
        drop(session);
        service.stop();

        // ---- 步骤 7：backup → 新目录 restore → 再读同一 release ----
        let backup_dir = self.work_dir.join("backup2");
        run_binary(
            &local_binary,
            &self.work_dir,
            &[
                "backup",
                "--data-dir",
                &data_dir.display().to_string(),
                "--out",
                &backup_dir.display().to_string(),
            ],
        )?;
        println!(
            "[步骤 7] backup → {}（停服后一致快照）",
            backup_dir.display()
        );

        let restored2 = self.work_dir.join("data2");
        run_binary(
            &local_binary,
            &self.work_dir,
            &[
                "restore",
                "--from",
                &backup_dir.display().to_string(),
                "--data-dir",
                &restored2.display().to_string(),
            ],
        )?;
        let mut service =
            Service::start(&local_binary, &self.work_dir, &restored2, Some(&self.log))?;
        let session = Session::login(&client, &service.base_url, &self.password)?;
        let second = read_release(&session)?;
        check_asset(
            &session,
            &second.model_asset_id,
            &second.model_sha256,
            "GLB(二次恢复)",
        )?;
        check_asset(
            &session,
            &second.pdf_asset_id,
            &second.pdf_sha256,
            "PDF(二次恢复)",
        )?;
        if second.manifest_sha256 != first_seen.manifest_sha256
            || second.model_sha256 != first_seen.model_sha256
            || second.pdf_sha256 != first_seen.pdf_sha256
        {
            bail!("二次恢复读取的 release 与首轮不一致：{second:?} != {first_seen:?}");
        }
        println!("[步骤 7] 二次恢复后同一 release 可读，manifest/模型/PDF sha256 与首轮一致");
        drop(session);
        service.stop();

        println!(
            "\n步骤汇总：1 冷目录 / 2 restore+init / 3 静态与路由 / 4 认证与资产 / \
                  5 断网读取 / 6 重启持久 / 7 备份恢复链 —— 全部通过。"
        );
        Ok(())
    }

    /// 步骤 5：断外网读取。macOS 用 sandbox-exec 拒绝非 localhost 出站；其它平台跳过
    /// （Linux 的离线证据由整条命令在 `--network none` 容器内运行提供）。
    fn check_offline(
        &self,
        client: &Client,
        binary: &Path,
        data_dir: &Path,
        first_seen: &SealedState,
    ) -> Result<()> {
        let sandbox = Path::new("/usr/bin/sandbox-exec");
        if self.skip_offline_sandbox {
            println!("[步骤 5] 已通过 --skip-offline-sandbox 跳过沙箱复检");
            return Ok(());
        }
        if !cfg!(target_os = "macos") || !sandbox.is_file() {
            println!(
                "[步骤 5] 本平台无 sandbox-exec：跳过沙箱复检（Linux 请用 \
                 `docker run --network none` 运行整条 smoke 作为离线性证据）"
            );
            return Ok(());
        }

        let profile = self.work_dir.join("offline.sb");
        std::fs::write(&profile, OFFLINE_SANDBOX_PROFILE)
            .with_context(|| format!("写入沙箱 profile 失败：{}", profile.display()))?;
        let _profile_guard = FileGuard(&profile);

        let mut service = Service::start(binary, &self.work_dir, data_dir, Some(&self.log))
            .context("在断网沙箱内启动服务失败")?;
        let session = Session::login(client, &service.base_url, &self.password)?;
        let release = read_release(&session)?;
        check_asset(
            &session,
            &release.model_asset_id,
            &release.model_sha256,
            "GLB(断网)",
        )?;
        check_asset(
            &session,
            &release.pdf_asset_id,
            &release.pdf_sha256,
            "PDF(断网)",
        )?;
        if release.manifest_sha256 != first_seen.manifest_sha256
            || release.model_sha256 != first_seen.model_sha256
            || release.pdf_sha256 != first_seen.pdf_sha256
        {
            bail!("断网沙箱内读取的数据与首轮不一致：{release:?} != {first_seen:?}");
        }
        let ready = session.get_json("/api/v1/health/ready")?;
        if ready["data"]["status"] != "ready" {
            bail!("断网沙箱内 /health/ready 异常：{ready}");
        }
        println!(
            "[步骤 5] 断外网（sandbox-exec：deny network-outbound，仅放行 localhost）读取通过：\
             已发布 release、GLB、PDF 可读且 sha256 一致，ready=ready"
        );
        drop(session);
        service.stop();
        Ok(())
    }
}

/// 首轮读取到的、后续步骤必须保持一致的状态。
#[derive(Debug)]
struct SealedState {
    manifest_sha256: String,
    model_sha256: String,
    pdf_sha256: String,
}

// ---------------------------------------------------------------------------
// HTTP 层
// ---------------------------------------------------------------------------

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("构造 HTTP 客户端失败")
}

struct Session {
    client: Client,
    base_url: String,
    cookie: String,
    csrf: String,
}

impl Session {
    fn login(client: &Client, base_url: &str, password: &str) -> Result<Self> {
        let body = serde_json::json!({ "password": password }).to_string();
        let response = client
            .post(format!("{base_url}/api/v1/auth/login"))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .context("登录请求失败")?;
        let status = response.status().as_u16();
        if status != 200 {
            let text = response.text().unwrap_or_default();
            bail!("登录应返回 200，实际 {status}：{text}");
        }
        let set_cookie =
            header_value(&response, "set-cookie").context("登录响应缺少 Set-Cookie")?;
        if !set_cookie.contains("HttpOnly") || !set_cookie.contains("SameSite=Strict") {
            bail!("会话 cookie 属性异常：{set_cookie}");
        }
        let cookie = set_cookie
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !cookie.starts_with("em_session=") {
            bail!("会话 cookie 名称异常：{cookie}");
        }
        let payload: serde_json::Value = response.json().context("登录响应不是 JSON")?;
        let csrf = payload["data"]["csrfToken"]
            .as_str()
            .context("登录响应缺少 csrfToken")?
            .to_owned();
        Ok(Self {
            client: client.clone(),
            base_url: base_url.to_owned(),
            cookie,
            csrf,
        })
    }

    fn get(&self, path: &str) -> Result<Response> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .header("cookie", &self.cookie)
            .send()
            .with_context(|| format!("GET {path} 失败"))
    }

    fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let response = self.get(path)?;
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        if status != 200 {
            bail!("GET {path} 应 200，实际 {status}：{text}");
        }
        serde_json::from_str(&text).with_context(|| format!("GET {path} 响应不是 JSON：{text}"))
    }

    fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
        let response = self
            .client
            .post(format!("{}{path}", self.base_url))
            .header("cookie", &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .with_context(|| format!("POST {path} 失败"))?;
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text }));
        Ok((status, parsed))
    }
}

/// 从备份 manifest 读取 blob 的 sha256 集合（恢复后读取的 sha 必须来自备份清单）。
struct BackupIndex {
    blob_sha256: BTreeSet<String>,
}

impl BackupIndex {
    fn read(backup: &Path) -> Result<Self> {
        let manifest_path = backup.join("manifest.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("读取备份 manifest 失败：{}", manifest_path.display()))?;
        let document: serde_json::Value =
            serde_json::from_str(&raw).context("备份 manifest 不是合法 JSON")?;
        let schema = document["schemaVersion"].as_str().unwrap_or_default();
        if schema != "manual_backup_v1" {
            bail!("备份 manifest schemaVersion 异常：{schema}");
        }
        let mut blob_sha256 = BTreeSet::new();
        for blob in document["blobs"]
            .as_array()
            .context("备份 manifest 缺少 blobs 数组")?
        {
            if let Some(sha) = blob["sha256"].as_str() {
                blob_sha256.insert(sha.to_owned());
            }
        }
        Ok(Self { blob_sha256 })
    }
}

/// 恢复后的 release 读取结果（manifest 声明的 sha 与实际下载内容的 sha 一并返回）。
#[derive(Debug)]
struct ReleaseData {
    item_id: String,
    release_id: String,
    manifest_sha256: String,
    model_asset_id: String,
    model_sha256: String,
    model_declared_sha256: String,
    pdf_asset_id: String,
    pdf_sha256: String,
    pdf_declared_sha256: String,
    preparation_id: String,
}

/// 通过 API 发现恢复数据中的物品与 release（不硬编码 ID；以恢复后数据为准）。
fn read_release(session: &Session) -> Result<ReleaseData> {
    let items = session.get_json("/api/v1/items")?;
    let item_id = items["data"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["id"].as_str())
        .context("备份中应至少有一个物品")?
        .to_owned();

    let releases = session.get_json(&format!("/api/v1/items/{item_id}/releases"))?;
    let release_id = releases["data"]
        .as_array()
        .and_then(|releases| releases.first())
        .and_then(|release| release["id"].as_str())
        .context("备份中应至少有一个已发布版本")?
        .to_owned();

    let detail = session.get_json(&format!("/api/v1/items/{item_id}/releases/{release_id}"))?;
    let data = &detail["data"];
    let manifest_sha256 = data["manifestSha256"]
        .as_str()
        .context("release 详情缺少 manifestSha256")?
        .to_owned();
    let manifest = &data["manifest"];
    let model_asset_id = manifest["model"]["assetId"]
        .as_str()
        .context("manifest 缺少 model.assetId")?
        .to_owned();
    let model_declared_sha256 = manifest["model"]["sha256"]
        .as_str()
        .context("manifest 缺少 model.sha256")?
        .to_owned();
    let document = manifest["documents"]
        .as_array()
        .and_then(|documents| documents.first())
        .context("manifest 缺少 documents")?;
    let pdf_asset_id = document["sourceAssetId"]
        .as_str()
        .context("manifest 缺少 documents[0].sourceAssetId")?
        .to_owned();
    let pdf_declared_sha256 = document["sourceSha256"]
        .as_str()
        .context("manifest 缺少 documents[0].sourceSha256")?
        .to_owned();
    let preparation_id = document["preparationId"]
        .as_str()
        .context("manifest 缺少 documents[0].preparationId")?
        .to_owned();

    let model_bytes = download_asset(session, &model_asset_id)?;
    let pdf_bytes = download_asset(session, &pdf_asset_id)?;

    Ok(ReleaseData {
        item_id,
        release_id,
        manifest_sha256,
        model_asset_id,
        model_sha256: sha256_bytes(&model_bytes),
        model_declared_sha256,
        pdf_asset_id,
        pdf_sha256: sha256_bytes(&pdf_bytes),
        pdf_declared_sha256,
        preparation_id,
    })
}

fn download_asset(session: &Session, asset_id: &str) -> Result<Vec<u8>> {
    let response = session.get(&format!("/api/v1/assets/{asset_id}/content"))?;
    let status = response.status().as_u16();
    if status != 200 {
        let text = response.text().unwrap_or_default();
        bail!("读取资产 {asset_id} 应 200，实际 {status}：{text}");
    }
    let bytes = response.bytes().context("读取资产响应体失败")?;
    if bytes.is_empty() {
        bail!("资产 {asset_id} 内容为空");
    }
    Ok(bytes.to_vec())
}

/// 资产的完整 GET / Range / HEAD / ETag 检查（§7 第 3、4 步的 Range/HEAD 要求）。
fn check_asset(
    session: &Session,
    asset_id: &str,
    expected_sha256: &str,
    label: &str,
) -> Result<()> {
    let url = format!("{}/api/v1/assets/{asset_id}/content", session.base_url);
    let client = &session.client;

    let full = client
        .get(&url)
        .header("cookie", &session.cookie)
        .send()
        .with_context(|| format!("{label} 完整读取失败"))?;
    let full_status = full.status().as_u16();
    let etag = header_value(&full, "etag").unwrap_or_default();
    let accept_ranges = header_value(&full, "accept-ranges").unwrap_or_default();
    let bytes = full.bytes().context("读取完整内容失败")?;
    let actual = sha256_bytes(&bytes);
    if full_status != 200 || actual != expected_sha256 {
        bail!(
            "{label} 完整读取异常：status={full_status} sha256={actual}（期望 {expected_sha256}）"
        );
    }
    if etag.trim_matches('"') != expected_sha256 || accept_ranges != "bytes" {
        bail!("{label} 响应头异常：etag={etag} accept-ranges={accept_ranges}");
    }
    println!("  [检查] {label} GET -> 200（sha256 与 manifest 一致，etag 一致）");

    let range = client
        .get(&url)
        .header("cookie", &session.cookie)
        .header("range", "bytes=0-99")
        .send()
        .with_context(|| format!("{label} Range 请求失败"))?;
    let range_status = range.status().as_u16();
    let content_range = header_value(&range, "content-range").unwrap_or_default();
    let range_bytes = range.bytes().context("读取 Range 内容失败")?;
    let expected_range = format!("bytes 0-99/{}", bytes.len());
    if range_status != 206 || content_range != expected_range || range_bytes.len() != 100 {
        bail!(
            "{label} Range 异常：status={range_status} content-range={content_range}\
             （期望 {expected_range}）长度 {}",
            range_bytes.len()
        );
    }
    if range_bytes.as_ref() != &bytes[..100] {
        bail!("{label} Range 内容与完整内容前缀不一致");
    }
    println!("  [检查] {label} GET Range bytes=0-99 -> 206 {content_range}（前缀一致）");

    let head = client
        .head(&url)
        .header("cookie", &session.cookie)
        .send()
        .with_context(|| format!("{label} HEAD 请求失败"))?;
    let head_status = head.status().as_u16();
    let head_length = header_value(&head, "content-length").unwrap_or_default();
    let head_bytes = head.bytes().context("读取 HEAD 响应失败")?;
    if head_status != 200 || head_length != bytes.len().to_string() || !head_bytes.is_empty() {
        bail!(
            "{label} HEAD 异常：status={head_status} content-length={head_length}（期望 {}）body 长度 {}",
            bytes.len(),
            head_bytes.len()
        );
    }
    println!("  [检查] {label} HEAD -> 200 content-length={head_length} 无 body");
    Ok(())
}

/// 正式包无 Provider 配置时，报价必须明确拒绝（409 + `PROVIDER_NOT_CONFIGURED`）。
fn check_unconfigured_rejection(
    session: &Session,
    item_id: &str,
    preparation_id: &str,
) -> Result<()> {
    let photos = session.get_json(&format!("/api/v1/items/{item_id}/photos"))?;
    let photo_ids: Vec<String> = photos["data"]
        .as_array()
        .map(|photos| {
            photos
                .iter()
                .filter_map(|photo| photo["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if photo_ids.len() < 2 {
        bail!("恢复数据应含 front + 侧视图两张照片，实际 {photo_ids:?}");
    }
    let body = serde_json::json!({
        "preparationId": preparation_id,
        "photoIds": photo_ids.iter().take(2).collect::<Vec<_>>(),
        "modelPreset": "tripo-h-v3.1-standard",
    });
    let (status, payload) =
        session.post_json(&format!("/api/v1/items/{item_id}/estimates"), &body)?;
    let code = payload["error"]["code"].as_str().unwrap_or_default();
    if status != 409 || code != NOT_CONFIGURED_CODE {
        bail!("未配置 Provider 时报价应 409 {NOT_CONFIGURED_CODE}，实际 {status} {payload}");
    }
    let message = payload["error"]["message"].as_str().unwrap_or_default();
    if !message.contains("未配置") {
        bail!("未配置 Provider 的拒绝信息不可读：{message}");
    }
    println!("  [检查] POST /items/{{id}}/estimates -> 409 {code}（明确拒绝，不用 mock 冒充）");
    Ok(())
}

/// 步骤 3：静态资源与路由（JS/CSS/字体/PDF vendor、嵌套路由刷新、未知 API/静态资源）。
fn check_static_surface(client: &Client, base_url: &str, index_html: &str) -> Result<()> {
    let assets = collect_asset_refs(index_html);
    let js = assets
        .iter()
        .find(|path| path.ends_with(".js") || path.ends_with(".mjs"))
        .context("首页未引用 JS 资源")?
        .clone();
    let css = assets
        .iter()
        .find(|path| path.ends_with(".css"))
        .context("首页未引用 CSS 资源")?
        .clone();
    for (path, expect_type) in [(&js, "javascript"), (&css, "css")] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .with_context(|| format!("请求 {path} 失败"))?;
        let status = response.status().as_u16();
        let content = content_type(&response);
        let bytes = response.bytes().context("读取静态资源失败")?;
        if status != 200 || !content.contains(expect_type) || bytes.is_empty() {
            bail!(
                "静态资源 {path} 异常：{status} {content}（{} 字节）",
                bytes.len()
            );
        }
        println!(
            "  [检查] GET {path} -> 200 {content}（{} 字节）",
            bytes.len()
        );
    }

    // PDF.js 运行资源：CMaps / standard_fonts / WASM / ICC（全部内嵌，运行时不下载）。
    let vendor_groups: [(&str, &[&str]); 4] = [
        (
            "CMaps",
            &[
                "/vendor/pdfjs/cmaps/78-EUC-H.bcmap",
                "/vendor/pdfjs/cmaps/Adobe-Japan1-UCS2.bcmap",
            ],
        ),
        (
            "standard_fonts",
            &[
                "/vendor/pdfjs/standard_fonts/FoxitFixed.pfb",
                "/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf",
            ],
        ),
        (
            "wasm",
            &[
                "/vendor/pdfjs/wasm/openjpeg.wasm",
                "/vendor/pdfjs/wasm/qcms_bg.wasm",
            ],
        ),
        ("iccs", &["/vendor/pdfjs/iccs/CGATS001Compat-v2-micro.icc"]),
    ];
    for (label, candidates) in vendor_groups {
        let mut served: Option<String> = None;
        for candidate in candidates {
            let response = client
                .get(format!("{base_url}{candidate}"))
                .send()
                .with_context(|| format!("请求 {candidate} 失败"))?;
            if response.status().as_u16() == 200 {
                let bytes = response.bytes().unwrap_or_default();
                served = Some(format!("{candidate}（{} 字节）", bytes.len()));
                break;
            }
        }
        match served {
            Some(entry) => println!("  [检查] PDF 运行资源·{label} -> 200 {entry}"),
            None => bail!("PDF 运行资源·{label} 全部候选不可服务：{candidates:?}"),
        }
    }

    // 入口引用的 JS chunk 与 pdf.worker（真实构建哈希名，从内嵌文件内容发现）。
    let worker = discover_pdf_worker(client, base_url, &js)?;
    println!("  [检查] PDF worker -> 200 {worker}");

    let nested = format!(
        "{base_url}/items/00000000-0000-7000-8000-000000000000/releases/00000000-0000-7000-8000-000000000001"
    );
    let response = client.get(&nested).send().context("嵌套路由刷新请求失败")?;
    let status = response.status().as_u16();
    let content = content_type(&response);
    if status != 200 || !content.starts_with("text/html") {
        bail!("嵌套路由刷新应返回 200 text/html，实际 {status} {content}");
    }
    println!("  [检查] GET 嵌套路由（/items/…/releases/…）刷新 -> 200 text/html（SPA 回退）");
    Ok(())
}

/// 文本中出现的资源引用（绝对 `/assets/…` 与相对 `./…` 两种形态，规范化为绝对路径）。
fn collect_asset_refs(text: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for marker in ["/assets/", "./"] {
        let mut rest = text;
        while let Some(position) = rest.find(marker) {
            let tail = &rest[position..];
            let end = tail
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric()
                        || matches!(character, '.' | '-' | '_' | '/' | '~'))
                })
                .unwrap_or(tail.len());
            let candidate = &tail[..end];
            let file = candidate.rsplit('/').next().unwrap_or_default();
            if file.ends_with(".js")
                || file.ends_with(".mjs")
                || file.ends_with(".css")
                || file.ends_with(".wasm")
            {
                let path = if candidate.starts_with('/') {
                    candidate.to_owned()
                } else {
                    format!("/assets/{}", candidate.trim_start_matches("./"))
                };
                out.insert(path);
            }
            rest = &rest[position + marker.len()..];
        }
    }
    out.into_iter().collect()
}

/// 是否是 Vite 带内容哈希的构建产物名（如 `PreparePage-D-3r--ih.js`、
/// `pdf.worker.min-Dswkl-cV.mjs`）：这类引用必须可服务；其余（如库源码里作为文案残留的
/// `./pdf.worker.mjs`）只在真实存在时才被采用。
fn is_hashed_chunk(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or_default();
    let Some((stem, extension)) = file.rsplit_once('.') else {
        return false;
    };
    if !matches!(extension, "js" | "mjs" | "css" | "wasm") {
        return false;
    }
    let Some((prefix, hash)) = stem.split_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && hash.len() >= 5
        && hash
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 沿入口 JS 的 chunk 引用找到 PDF worker（文件名带构建哈希，不硬编码）。
fn discover_pdf_worker(client: &Client, base_url: &str, entry_js: &str) -> Result<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = vec![entry_js.to_owned()];
    let mut worker: Option<String> = None;
    while let Some(path) = queue.pop() {
        if !seen.insert(path.clone()) || seen.len() > 40 {
            continue;
        }
        let is_worker_candidate = path.contains("pdf.worker");
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .with_context(|| format!("请求 {path} 失败"))?;
        let status = response.status().as_u16();
        if status != 200 {
            // 只有"构建哈希名"的引用必须存在；库源码里的路径文案（如 `./pdf.worker.mjs`）
            // 返回 404 属正常，不据此失败。
            if is_hashed_chunk(&path) {
                bail!("内嵌资源 {path} 应 200，实际 {status}（包内构建产物不完整）");
            }
            continue;
        }
        if is_worker_candidate {
            worker = Some(path.clone());
        }
        let body = response.text().unwrap_or_default();
        for reference in collect_asset_refs(&body) {
            if !seen.contains(&reference) {
                queue.push(reference);
            }
        }
    }
    worker.context("在内嵌 JS 中未发现 pdf.worker 资源引用（PDF 运行资源缺失？）")
}

// ---------------------------------------------------------------------------
// 进程与文件工具
// ---------------------------------------------------------------------------

/// 在给定 data-dir 上启动服务（工作目录 = 冒烟临时目录；环境清空，不继承 Provider 配置）。
struct Service {
    child: Option<Child>,
    base_url: String,
    pid: u32,
}

impl Service {
    fn start(
        binary: &Path,
        work_dir: &Path,
        data_dir: &Path,
        log: Option<&LogBuffer>,
    ) -> Result<Self> {
        let mut command = Command::new(binary);
        command
            .arg("serve")
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--listen")
            .arg("127.0.0.1:0")
            .current_dir(work_dir)
            // 关键：清空环境，避免继承 EM_* / Provider 密钥或 fixture 放行开关；
            // 工作目录不在仓库内，服务拿不到 config.toml（启动日志会记录未配置 Provider）。
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("无法启动 {}", binary.display()))?;
        let pid = child.id();
        let stdout_lines = spawn_line_reader(&mut child);
        let stderr = Box::new(child.stderr.take().expect("stderr 已管道化"));
        spawn_log_collector(stderr, log.cloned());
        match wait_for_listening(&stdout_lines) {
            Ok(base_url) => Ok(Self {
                child: Some(child),
                base_url,
                pid,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            println!("  [清理] 已结束自启动的服务进程 PID {}", self.pid);
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 删除临时 profile 文件的守卫（失败路径也清理）。
struct FileGuard<'a>(&'a Path);

impl Drop for FileGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

fn run_binary(binary: &Path, work_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(binary)
        .args(args)
        .current_dir(work_dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("无法执行 {} {}", binary.display(), args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "命令失败（退出码 {:?}）：{} {}\nstdout: {}\nstderr: {}",
            output.status.code(),
            binary.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn copy_binary(source: &Path, destination: &Path) -> Result<()> {
    std::fs::copy(source, destination).with_context(|| {
        format!(
            "拷贝二进制失败：{} → {}",
            source.display(),
            destination.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn write_password_file(path: &Path, password: &str) -> Result<()> {
    std::fs::write(path, format!("{password}\n"))
        .with_context(|| format!("写入临时密码文件失败：{}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// 复核服务进程没有派生子进程（Node/Python 等运行时不是本产品的运行依赖）。
fn check_no_child_processes(pid: u32) {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let children = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if children.is_empty() {
                println!("  [检查] 服务进程无子进程（不依赖 Node/Python/外部程序）");
            } else {
                println!("  [注意] 服务进程存在子进程：{children}（请人工复核是否为运行依赖）");
            }
        }
        _ => println!("  [检查] pgrep 不可用，跳过子进程复核"),
    }
}

fn temp_work_dir(kind: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "everything-manual-{kind}-{}-{nanos}",
        std::process::id()
    ))
}

type LineReader = Receiver<String>;

fn spawn_line_reader(child: &mut Child) -> LineReader {
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().expect("stdout 已管道化");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// 收集服务 stderr（结构化日志）到共享缓冲；无缓冲时仅排空管道避免子进程阻塞。
fn spawn_log_collector(stream: Box<dyn Read + Send>, buffer: Option<LogBuffer>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Some(buffer) = &buffer
                && let Ok(mut lines) = buffer.lock()
                && lines.len() < 5000
            {
                lines.push(line);
            }
        }
    });
}

fn print_log_tail(buffer: &LogBuffer, lines: usize) {
    let log = match buffer.lock() {
        Ok(log) => log.join("\n"),
        Err(_) => return,
    };
    if log.trim().is_empty() {
        return;
    }
    eprintln!("--- 服务日志尾部（最后 {lines} 行）---");
    let all: Vec<&str> = log.lines().collect();
    for line in all.iter().skip(all.len().saturating_sub(lines)) {
        eprintln!("{line}");
    }
}

/// 等待并解析启动输出的 `listening on http://<addr>` 行。
fn wait_for_listening(lines: &LineReader) -> Result<String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("启动输出等待超时（未打印 listening 行）")?;
        let line = lines
            .recv_timeout(remaining)
            .context("启动输出等待超时（未打印 listening 行）")?;
        if let Some(address) = line.strip_prefix("listening on ") {
            let address = address.trim();
            if !address.starts_with("http://") {
                bail!("listening 行格式异常：{line}");
            }
            return Ok(address.trim_end_matches('/').to_owned());
        }
        println!("  (服务输出) {line}");
    }
}

fn fetch_index(client: &Client, base_url: &str) -> Result<String> {
    let response = client.get(base_url).send().context("请求首页失败")?;
    let status = response.status().as_u16();
    let content = content_type(&response);
    let body = response.text().context("读取首页失败")?;
    if status != 200 || !content.starts_with("text/html") {
        bail!("首页应 200 text/html，实际 {status} {content}");
    }
    if !body.contains("id=\"root\"") {
        bail!("内嵌 index.html 缺少挂载点 id=\"root\"");
    }
    println!(
        "  [检查] GET / -> 200 text/html（内嵌 index.html，{} 字节）",
        body.len()
    );
    Ok(body)
}

fn content_type(response: &Response) -> String {
    header_value(response, "content-type").unwrap_or_default()
}

fn header_value(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
