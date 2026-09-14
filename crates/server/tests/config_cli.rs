//! T02 集成测试：CLI 分派、配置优先级、日志脱敏、data-dir 排他锁。
//!
//! 覆盖的验收条件（PRD 修订 1）：
//! - AC-005：`init/serve/check` 分派、未知配置键报错、缺密钥不启动 mock、
//!   第二个进程持同一 data-dir 失败退出、日志中不出现敏感字段、backup/restore 的 CLI
//!   冒烟与稳定退出码（T20 起为真实实现；完整覆盖见 `backup_restore.rs`）；
//! - AC-006：非 loopback 无 TLS/可信代理时 serve 拒绝启动；check 不发起任何外部 HTTP；
//! - AC-012（配置侧）：Provider 未配置时服务可启动、状态如实标注，无 mock 回退。
//!
//! 所有用例通过真实二进制子进程 + 临时目录执行；测试用假凭据（canary），不接触真实密钥。

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_everything-manual");
const LOG_FILE: &str = "logs/everything-manual.log";

// ---------------------------------------------------------------------------
// 基础工具
// ---------------------------------------------------------------------------

/// 自动清理的临时目录（每个用例一个，互不干扰）。
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "em-config-cli-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("创建测试临时目录");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct CmdOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

/// 在指定工作目录运行二进制（清空环境变量，保证用例确定性）。
fn run(dir: &Path, args: &[&str]) -> CmdOutput {
    run_with_env(dir, args, &[])
}

fn run_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("二进制应可启动");
    let output = CmdOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    assert!(
        !output.stderr.contains("panicked at"),
        "错误路径不得 panic：{output:?}"
    );
    output
}

/// 写受限文件（0600）。
fn write_restricted(path: &Path, content: &str) {
    std::fs::write(path, content).expect("写受限文件");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).expect("写文件");
}

fn read_log(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join(LOG_FILE)).unwrap_or_default()
}

/// `init` 一个 data-dir，返回其路径；密码来自 0600 文件。
fn init_data_dir(dir: &TestDir) -> PathBuf {
    let password = dir.join("pw.txt");
    write_restricted(&password, "test-password-123\n");
    let out = run(
        dir.path(),
        &[
            "init",
            "--data-dir",
            dir.join("data").to_str().unwrap(),
            "--password-file",
            password.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status, 0, "init 应成功：{out:?}");
    dir.join("data")
}

// ---------------------------------------------------------------------------
// serve 子进程
// ---------------------------------------------------------------------------

struct ServeProcess {
    child: Child,
    addr: String,
    stdout_lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
}

impl ServeProcess {
    fn start(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Self {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .current_dir(dir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("serve 应能启动");

        let (sender, receiver) = mpsc::channel::<String>();
        let stdout = child.stdout.take().expect("stdout 已管道化");
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_sink = stderr.clone();
        let stderr_pipe = child.stderr.take().expect("stderr 已管道化");
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr_pipe).read_to_string(&mut text);
            *stderr_sink.lock().unwrap() = text;
        });

        let deadline = Instant::now() + Duration::from_secs(15);
        let addr = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("等待 listening 行超时");
            match receiver.recv_timeout(remaining) {
                Ok(line) => {
                    if let Some(addr) = line.strip_prefix("listening on http://") {
                        break addr.trim().to_owned();
                    }
                }
                Err(error) => panic!(
                    "serve 未打印 listening 行（{error}）；stderr：{}",
                    stderr.lock().unwrap()
                ),
            }
        };

        // `listening on` 打印在 SIGTERM 处理器注册之前（见 common::settle_after_listening_line
        // 与 BUG-013）：本文件既有用例通常先有 HTTP 交互、未命中该窗口，但 settle 使
        // `start()` 返回后即可安全发信号，不依赖"调用方恰好先做了别的 I/O"。只加等待，不改断言。
        common::settle_after_listening_line();

        Self {
            child,
            addr,
            stdout_lines: receiver,
            stderr,
        }
    }

    fn addr(&self) -> &str {
        &self.addr
    }

    /// 收集当前已经输出的 stdout（非阻塞），用于断言启动日志。
    fn drain_stdout(&self) -> String {
        let mut collected = String::new();
        while let Ok(line) = self.stdout_lines.try_recv() {
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    }

    fn stderr_text(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// SIGKILL 并回收（模拟强制结束；锁由操作系统释放）。
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// SIGTERM 并等待优雅退出，返回退出码。
    fn terminate(mut self) -> i32 {
        #[cfg(unix)]
        {
            let pid = self.child.id().to_string();
            let _ = Command::new("kill")
                .args(["-TERM", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return status.code().unwrap_or(-1);
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("SIGTERM 后未在 10 秒内退出");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// 极简 HTTP GET（只用标准库，避免测试依赖真实 HTTP 客户端）。
fn http_get(addr: &str, target: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("连接测试服务");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .expect("写请求");
    let mut raw = String::new();
    let _ = stream.read_to_string(&mut raw);
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status, raw)
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[test]
fn init_creates_structure_and_never_leaks_password() {
    const CANARY: &str = "canary-password-2f31e9";
    let dir = TestDir::new("init");
    let password = dir.join("pw.txt");
    write_restricted(&password, &format!("{CANARY}\n"));

    let out = run(
        dir.path(),
        &["init", "--data-dir", "data", "--password-file", "pw.txt"],
    );
    assert_eq!(out.status, 0, "{out:?}");
    let data = dir.join("data");
    for sub in ["tmp", "logs", "blobs"] {
        assert!(data.join(sub).is_dir(), "缺少 {sub}/：{out:?}");
    }
    assert!(data.join("lock").is_file());
    // T03 起 `init` 创建并迁移数据库（T02 曾刻意不建库，避免无 schema 空库；
    // ADR-011 注 9 已说明"数据库由 T03 迁移创建"，本断言随 T03 更新）。
    assert!(
        data.join("manual.sqlite3").is_file(),
        "init 必须创建并迁移数据库（T03）：{out:?}"
    );
    // T19 起为 schema v7（0007_release_manifest；迁移只追加，见 ADR-009）。
    // 事实更新（T19 交付回合，2026-09-12）：版本号随新增迁移变化，断言口径不变。
    assert!(out.stdout.contains("schema v7"), "{out:?}");

    // 敏感值不得出现在 stdout/stderr 与日志文件中。
    assert!(!out.stdout.contains(CANARY) && !out.stderr.contains(CANARY));
    let log = read_log(&data);
    assert!(log.contains("\"event\":\"init\""), "{log}");
    assert!(!log.contains(CANARY), "密码进入日志：{log}");
    // T04 起 init 持久化管理员凭据（Argon2id 哈希入 admins 表）；
    // 明文仍不出现在任何输出/日志中。
    assert!(
        out.stdout.contains("管理员凭据：已创建"),
        "init 必须明确管理员凭据已写入：{out:?}"
    );
    assert!(
        out.stdout.contains("Argon2id"),
        "init 必须说明哈希参数：{out:?}"
    );

    // 幂等：再次 init 成功且不破坏已有内容。
    write_text(&data.join("tmp/keep.txt"), "keep");
    let again = run(
        dir.path(),
        &["init", "--data-dir", "data", "--password-file", "pw.txt"],
    );
    assert_eq!(again.status, 0, "{again:?}");
    assert!(data.join("tmp/keep.txt").exists(), "init 不得清空 tmp/");
}

#[test]
fn init_requires_password_source_and_rejects_argv_password() {
    const CANARY: &str = "canary-argv-pass-93ba";
    let dir = TestDir::new("init-password");

    // 非交互环境且没有 --password-file：必须是可读错误，且不创建 data-dir。
    let missing = run(dir.path(), &["init", "--data-dir", "data"]);
    assert_eq!(missing.status, 2, "{missing:?}");
    assert!(
        missing.stderr.contains("--password-file"),
        "应提示使用受限文件：{missing:?}"
    );
    assert!(!dir.join("data").exists(), "密码无效时不得创建 data-dir");

    // 密码不能出现在命令行（不进 shell history）。
    let argv = run(
        dir.path(),
        &["init", "--data-dir", "data", "--password", CANARY],
    );
    assert_eq!(argv.status, 2, "{argv:?}");
    assert!(argv.stderr.contains("--password"), "{argv:?}");

    // 宽权限密码文件被拒绝。
    let loose = dir.join("loose.txt");
    std::fs::write(&loose, "test-password-123\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(unix)]
    {
        let out = run(
            dir.path(),
            &["init", "--data-dir", "data", "--password-file", "loose.txt"],
        );
        assert_eq!(out.status, 3, "{out:?}");
        assert!(out.stderr.contains("chmod 600"), "{out:?}");
        assert!(!dir.join("data").exists());
    }

    // 密码文件不存在。
    let absent = run(
        dir.path(),
        &[
            "init",
            "--data-dir",
            "data",
            "--password-file",
            "absent.txt",
        ],
    );
    assert_eq!(absent.status, 3, "{absent:?}");
}

// ---------------------------------------------------------------------------
// 子命令分派与错误路径
// ---------------------------------------------------------------------------

#[test]
fn usage_errors_are_readable_and_nonzero() {
    let dir = TestDir::new("usage");

    let unknown = run(dir.path(), &["frobnicate"]);
    assert_eq!(unknown.status, 2, "{unknown:?}");
    assert!(unknown.stderr.contains("frobnicate"), "{unknown:?}");
    assert!(unknown.stderr.contains("Usage"), "{unknown:?}");

    let no_args = run(dir.path(), &[]);
    assert_eq!(no_args.status, 2, "{no_args:?}");

    // 没有任何来源的 data-dir：配置错误（3），给出修复建议。
    let no_data_dir = run(dir.path(), &["serve"]);
    assert_eq!(no_data_dir.status, 3, "{no_data_dir:?}");
    assert!(no_data_dir.stderr.contains("data_dir"), "{no_data_dir:?}");

    // data-dir 未初始化：serve 明确要求先 init（不隐式创建结构）。
    let missing = run(dir.path(), &["serve", "--data-dir", "missing"]);
    assert_eq!(missing.status, 4, "{missing:?}");
    assert!(missing.stderr.contains("init"), "{missing:?}");

    let check_missing = run(dir.path(), &["check", "--data-dir", "missing"]);
    assert_eq!(check_missing.status, 4, "{check_missing:?}");

    // 目录存在但结构不完整（未 init）：serve 与 check 都指向 init 修复。
    std::fs::create_dir_all(dir.join("empty-dir")).unwrap();
    let structured = run(dir.path(), &["serve", "--data-dir", "empty-dir"]);
    assert_eq!(structured.status, 4, "{structured:?}");
    assert!(
        structured.stderr.contains("结构不完整") && structured.stderr.contains("init"),
        "{structured:?}"
    );
    let check_structured = run(dir.path(), &["check", "--data-dir", "empty-dir"]);
    assert_eq!(check_structured.status, 4, "{check_structured:?}");
}

#[test]
fn unknown_config_keys_are_rejected_at_any_level() {
    let dir = TestDir::new("unknown-key");
    let data = init_data_dir(&dir);
    write_text(
        &data.join("config.toml"),
        "listen = \"127.0.0.1:9999\"\ntotally_unknown_key = 1\n",
    );
    let top = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(top.status, 3, "{top:?}");
    assert!(top.stderr.contains("totally_unknown_key"), "{top:?}");

    write_text(
        &data.join("config.toml"),
        "[providers.tripo]\nbogus_field = \"x\"\n",
    );
    let nested = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(nested.status, 3, "{nested:?}");
    assert!(nested.stderr.contains("bogus_field"), "{nested:?}");

    // 合法配置通过，并如实展示有效值。
    write_text(&data.join("config.toml"), "listen = \"127.0.0.1:9999\"\n");
    let ok = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(ok.status, 0, "{ok:?}");
    assert!(ok.stdout.contains("listen = 127.0.0.1:9999"), "{ok:?}");
    assert!(ok.stdout.contains("结果：check 通过"), "{ok:?}");
}

#[test]
fn config_precedence_cli_env_toml_default() {
    let dir = TestDir::new("precedence");
    let data = init_data_dir(&dir);

    // 默认值。
    let default_out = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(default_out.status, 0, "{default_out:?}");
    assert!(
        default_out.stdout.contains("listen = 127.0.0.1:8080"),
        "{default_out:?}"
    );

    // TOML > 默认。
    write_text(&data.join("config.toml"), "listen = \"127.0.0.1:1111\"\n");
    let toml_out = run(dir.path(), &["check", "--data-dir", "data"]);
    assert!(
        toml_out.stdout.contains("listen = 127.0.0.1:1111"),
        "{toml_out:?}"
    );

    // 环境变量 > TOML。
    let env_out = run_with_env(
        dir.path(),
        &["check", "--data-dir", "data"],
        &[("EM_LISTEN", "127.0.0.1:2222")],
    );
    assert!(
        env_out.stdout.contains("listen = 127.0.0.1:2222"),
        "{env_out:?}"
    );

    // CLI > 环境变量。
    let cli_out = run_with_env(
        dir.path(),
        &["check", "--data-dir", "data", "--listen", "127.0.0.1:3333"],
        &[("EM_LISTEN", "127.0.0.1:2222")],
    );
    assert!(
        cli_out.stdout.contains("listen = 127.0.0.1:3333"),
        "{cli_out:?}"
    );

    // data_dir 方向：TOML 指向另一个目录，但 CLI > TOML、环境变量 > TOML。
    let other = dir.join("other-data");
    let other_pw = dir.join("other-pw.txt");
    write_restricted(&other_pw, "test-password-456\n");
    let other_init = run(
        dir.path(),
        &[
            "init",
            "--data-dir",
            "other-data",
            "--password-file",
            "other-pw.txt",
        ],
    );
    assert_eq!(other_init.status, 0, "{other_init:?}");
    write_text(
        &data.join("config.toml"),
        &format!(
            "data_dir = \"{}\"\n",
            other.to_str().unwrap().replace('\\', "\\\\")
        ),
    );
    // 注：macOS 下 /var 是指向 /private/var 的符号链接，进程 getcwd 与测试进程可能
    // 得到不同前缀；断言统一 canonicalize 两侧后再比较。
    let cli_wins = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(
        reported_data_dir(&cli_wins.stdout),
        std::fs::canonicalize(&data).unwrap(),
        "CLI --data-dir 应覆盖 TOML：{cli_wins:?}"
    );
    let env_wins = run_with_env(
        dir.path(),
        &[
            "check",
            "--config",
            data.join("config.toml").to_str().unwrap(),
        ],
        &[("EM_DATA_DIR", other.to_str().unwrap())],
    );
    assert_eq!(
        reported_data_dir(&env_wins.stdout),
        std::fs::canonicalize(&other).unwrap(),
        "环境变量 EM_DATA_DIR 应覆盖 TOML：{env_wins:?}"
    );
}

/// 从 `check` 输出中取出 `data_dir = <path>` 行（canonicalize 后比较）。
fn reported_data_dir(stdout: &str) -> PathBuf {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("data_dir = "))
        .unwrap_or_else(|| panic!("check 输出应包含 data_dir 行：{stdout}"));
    let value = line.trim_start_matches("data_dir = ").trim();
    std::fs::canonicalize(value).unwrap_or_else(|_| PathBuf::from(value))
}

// ---------------------------------------------------------------------------
// 缺密钥语义（REQ-007 / AC-012 配置侧）
// ---------------------------------------------------------------------------

#[test]
fn missing_provider_keys_start_without_mock_fallback() {
    const TRIPO_CANARY: &str = "canary-tripo-key-1a2b";
    const MANUAL_AI_CANARY: &str = "canary-manual-ai-key-3c4d";
    let dir = TestDir::new("no-provider");
    let data = init_data_dir(&dir);

    // 无任何 Provider 配置：check 如实标注“未配置”，不出现“已配置”。
    let check = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(check.status, 0, "{check:?}");
    assert!(
        check.stdout.contains("providers.tripo = 未配置"),
        "{check:?}"
    );
    assert!(
        check.stdout.contains("providers.manual_ai = 未配置"),
        "{check:?}"
    );
    assert!(!check.stdout.contains("已配置"), "{check:?}");

    // 缺少密钥时服务仍可启动（浏览已有资料），且日志如实标注、无 mock 回退。
    let serve = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
        &[],
    );
    let (status, _) = http_get(serve.addr(), "/api/v1/health/live");
    assert_eq!(status, 200, "缺密钥时服务必须可启动");
    let stderr_before_stop = serve.stderr_text();
    let code = serve.terminate();
    assert_eq!(code, 0, "SIGTERM 应优雅退出；stderr：{stderr_before_stop}");

    let log = read_log(&data);
    assert!(
        log.contains("\"event\":\"provider_not_configured\"")
            && log.contains("\"provider\":\"tripo\"")
            && log.contains("\"provider\":\"manual_ai\""),
        "缺密钥必须如实记录为未配置：{log}"
    );
    assert!(
        !log.contains("provider_configured"),
        "无密钥时不得出现任何 Provider 已配置事件：{log}"
    );
    assert!(
        log.contains("\"event\":\"serve_stop\""),
        "优雅退出应记录停止事件：{log}"
    );

    // api_key_env 已命名但环境变量未设置 → 仍然未配置（不存在隐式默认或 mock）。
    write_text(
        &data.join("config.toml"),
        "[providers.tripo]\napi_key_env = \"EM_TEST_TRIPO_KEY\"\n\n\
         [providers.manual_ai]\nmodel = \"test-model\"\napi_key_env = \"EM_TEST_MANUAL_AI_KEY\"\n",
    );
    let unset = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(unset.status, 0, "{unset:?}");
    assert!(
        unset.stdout.contains("providers.tripo = 未配置")
            && unset.stdout.contains("providers.manual_ai = 未配置"),
        "api_key_env 未导出时不得视为已配置：{unset:?}"
    );

    // 环境注入后可配置；密钥只出现在注入处，不进入输出与日志。
    let injected = run_with_env(
        dir.path(),
        &["check", "--data-dir", "data"],
        &[
            ("EM_TEST_TRIPO_KEY", TRIPO_CANARY),
            ("EM_TEST_MANUAL_AI_KEY", MANUAL_AI_CANARY),
        ],
    );
    assert_eq!(injected.status, 0, "{injected:?}");
    assert!(
        injected.stdout.contains("providers.tripo = 已配置")
            && injected.stdout.contains("providers.manual_ai = 已配置"),
        "{injected:?}"
    );
    assert!(
        !injected.stdout.contains(TRIPO_CANARY) && !injected.stdout.contains(MANUAL_AI_CANARY),
        "密钥不得出现在 check 输出：{injected:?}"
    );
    let log = read_log(&data);
    assert!(
        !log.contains(TRIPO_CANARY) && !log.contains(MANUAL_AI_CANARY),
        "{log}"
    );
}

// ---------------------------------------------------------------------------
// data-dir 排他锁
// ---------------------------------------------------------------------------

#[test]
fn second_process_on_same_data_dir_fails_fast() {
    let dir = TestDir::new("lock");
    let data = init_data_dir(&dir);

    let serve = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
        &[],
    );

    // 第二个 serve：必须非零退出（5），不得打印 listening。
    let second = run(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
    );
    assert_eq!(second.status, 5, "{second:?}");
    assert!(
        second.stderr.contains("正被另一个进程使用"),
        "错误信息应说明锁冲突：{second:?}"
    );
    assert!(!second.stdout.contains("listening on"), "{second:?}");

    // init 同样需要锁：不能在服务运行中并发初始化。
    let pw = dir.join("pw.txt");
    let init_while_locked = run(
        dir.path(),
        &[
            "init",
            "--data-dir",
            "data",
            "--password-file",
            pw.to_str().unwrap(),
        ],
    );
    assert_eq!(init_while_locked.status, 5, "{init_while_locked:?}");

    // check 在服务运行中给出警告但仍完成（锁被占用是合法运行状态）。
    let check = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(check.status, 0, "{check:?}");
    assert!(check.stdout.contains("排他锁：警告"), "{check:?}");

    serve.kill();

    // 进程结束后锁自动释放（含 SIGKILL）。
    let after = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(after.status, 0, "{after:?}");
    assert!(after.stdout.contains("排他锁：通过"), "{after:?}");

    // 同一个 data-dir 可以再次启动（新监听端口）。
    let restarted = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
        &[],
    );
    let (status, _) = http_get(restarted.addr(), "/api/v1/health/live");
    assert_eq!(status, 200);
    restarted.kill();
    let _ = data;
}

// ---------------------------------------------------------------------------
// check 不发起外部 HTTP（AC-006）
// ---------------------------------------------------------------------------

#[test]
fn check_and_serve_startup_make_no_external_http_requests() {
    const TRIPO_CANARY: &str = "canary-check-tripo-key";
    let dir = TestDir::new("no-http");
    let data = init_data_dir(&dir);

    // 计数监听器：任何连接都会被接受；断言连接数为 0。
    let counter = TcpListener::bind("127.0.0.1:0").unwrap();
    counter.set_nonblocking(true).unwrap();
    let counter_addr = counter.local_addr().unwrap();
    let base_url = format!("http://{counter_addr}/v3");

    let price_catalog = dir.join("prices.toml");
    // T11：价格目录必须是可解析的目录（缺项会让 check/serve 退出码 3）。
    write_text(
        &price_catalog,
        "version = \"2026-09-11\"\nsnapshot_date = \"2026-09-11\"\n\n[[tripo.presets]]\n\
         preset = \"tripo-h-v3.1-standard\"\nmodel = \"v3.1-20260211\"\ncredits = \"30\"\n",
    );

    write_text(
        &data.join("config.toml"),
        &format!(
            "price_catalog_path = \"{}\"\n\n[providers.tripo]\nbase_url = \"{base_url}\"\n\
             api_key_env = \"EM_TEST_TRIPO_KEY\"\n\n[providers.manual_ai]\nbase_url = \"{base_url}\"\n\
             model = \"test-model\"\napi_key_env = \"EM_TEST_MANUAL_AI_KEY\"\n",
            price_catalog.display()
        ),
    );

    let envs: &[(&str, &str)] = &[
        ("EM_TEST_TRIPO_KEY", TRIPO_CANARY),
        ("EM_TEST_MANUAL_AI_KEY", "canary-check-manual-ai"),
    ];

    let check = run_with_env(dir.path(), &["check", "--data-dir", "data"], envs);
    assert_eq!(check.status, 0, "{check:?}");
    assert!(
        check.stdout.contains("本命令不发起任何网络请求"),
        "{check:?}"
    );
    assert!(
        check.stdout.contains("providers.tripo = 已配置"),
        "本用例需要 Provider 处于“已配置”以证明 check 仍不外呼：{check:?}"
    );

    // 启动 serve 也不应在启动阶段访问 Provider 端点。
    let serve = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
        envs,
    );
    let (status, _) = http_get(serve.addr(), "/api/v1/health/live");
    assert_eq!(status, 200);
    serve.kill();

    // 给潜在的后台线程一点时间，再断言零连接。
    std::thread::sleep(Duration::from_millis(300));
    match counter.accept() {
        Ok((stream, peer)) => {
            panic!("check/serve 启动不得访问 Provider 端点，但收到来自 {peer} 的连接（{stream:?}）")
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(error) => panic!("计数监听器异常：{error}"),
    }
}

// ---------------------------------------------------------------------------
// 监听安全与 TLS 边界（AC-006）
// ---------------------------------------------------------------------------

#[test]
fn non_loopback_listen_requires_tls_or_trusted_proxy() {
    let dir = TestDir::new("listen-security");
    let data = init_data_dir(&dir);

    // 非 loopback、无 TLS、无可信代理：serve 拒绝启动，check 同样失败。
    let serve = run(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "0.0.0.0:0"],
    );
    assert_eq!(serve.status, 6, "{serve:?}");
    assert!(serve.stderr.contains("trusted_proxy_cidrs"), "{serve:?}");
    assert!(!serve.stdout.contains("listening on"), "{serve:?}");

    let check = run(
        dir.path(),
        &["check", "--data-dir", "data", "--listen", "0.0.0.0:0"],
    );
    assert_eq!(check.status, 6, "{check:?}");

    // 明确配置可信反向代理后允许启动（仍然不默认相信 X-Forwarded-*）。
    write_text(
        &data.join("config.toml"),
        "trusted_proxy_cidrs = [\"127.0.0.0/8\"]\n",
    );
    let proxied = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "0.0.0.0:0"],
        &[],
    );
    // 监听在 0.0.0.0 上：通过 loopback 地址访问实际端口。
    let port = proxied.addr().rsplit(':').next().unwrap().to_owned();
    let (status, _) = http_get(&format!("127.0.0.1:{port}"), "/api/v1/health/live");
    assert_eq!(status, 200);
    proxied.kill();

    // 非法 CIDR 是配置错误。
    write_text(
        &data.join("config.toml"),
        "trusted_proxy_cidrs = [\"10.0.0.0/33\"]\n",
    );
    let bad_cidr = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(bad_cidr.status, 3, "{bad_cidr:?}");

    // TLS 必须成对配置。
    write_text(
        &data.join("config.toml"),
        "[tls]\ncert_file = \"cert.pem\"\n",
    );
    let half_tls = run(dir.path(), &["check", "--data-dir", "data"]);
    assert_eq!(half_tls.status, 3, "{half_tls:?}");
    assert!(half_tls.stderr.contains("成对"), "{half_tls:?}");

    // 完整 TLS 配置：内置 TLS 监听尚未实现 → 明确拒绝，不静默降级为明文。
    write_text(
        &dir.join("cert.pem"),
        "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
    );
    write_text(
        &dir.join("key.pem"),
        "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n",
    );
    write_text(
        &data.join("config.toml"),
        &format!(
            "[tls]\ncert_file = \"{}\"\nkey_file = \"{}\"\n",
            dir.join("cert.pem").display(),
            dir.join("key.pem").display()
        ),
    );
    let tls = run(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
    );
    assert_eq!(tls.status, 6, "{tls:?}");
    assert!(tls.stderr.contains("尚未实现"), "{tls:?}");
    assert!(!tls.stdout.contains("listening on"), "{tls:?}");
}

// ---------------------------------------------------------------------------
// 日志脱敏（AC-005 / §5.7）
// ---------------------------------------------------------------------------

#[test]
fn logs_never_contain_secrets_or_query_strings() {
    const PASSWORD_CANARY: &str = "canary-password-log-5e6f";
    const KEY_CANARY: &str = "canary-api-key-log-7a8b";
    const SIGNED_URL_CANARY: &str = "canary-signed-url-9c0d";
    let dir = TestDir::new("log-redaction");
    let data = dir.join("data");
    let password = dir.join("pw.txt");
    write_restricted(&password, &format!("{PASSWORD_CANARY}\n"));
    let init = run(
        dir.path(),
        &["init", "--data-dir", "data", "--password-file", "pw.txt"],
    );
    assert_eq!(init.status, 0, "{init:?}");
    assert!(!init.stdout.contains(PASSWORD_CANARY) && !init.stderr.contains(PASSWORD_CANARY));

    write_text(
        &data.join("config.toml"),
        "[providers.tripo]\napi_key_env = \"EM_TEST_TRIPO_KEY\"\n",
    );

    let serve = ServeProcess::start(
        dir.path(),
        &["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"],
        &[("EM_TEST_TRIPO_KEY", KEY_CANARY), ("RUST_LOG", "debug")],
    );
    // 带查询串的请求（模拟签名 URL 形态）：查询串不得进入日志。
    let (status, _) = http_get(
        serve.addr(),
        &format!("/api/v1/health/live?token={SIGNED_URL_CANARY}&expires=1"),
    );
    assert_eq!(status, 200);
    // 一次统一错误响应：日志应带 errorCode（不含堆栈）。
    let (not_found, _) = http_get(serve.addr(), "/api/unknown?token=SECOND-CANARY");
    assert_eq!(not_found, 404);
    let stdout = serve.drain_stdout();
    serve.kill();

    let log = read_log(&data);
    for canary in [PASSWORD_CANARY, KEY_CANARY, SIGNED_URL_CANARY] {
        assert!(!log.contains(canary), "敏感值进入日志：{canary}\n{log}");
        assert!(
            !stdout.contains(canary),
            "敏感值进入 stdout：{canary}\n{stdout}"
        );
    }
    assert!(!log.contains("token="), "查询串不得进入日志：{log}");

    // 请求日志必须包含脱敏后的必要上下文（requestId/耗时/状态码/错误码）。
    assert!(log.contains("\"message\":\"http_request\""), "{log}");
    assert!(log.contains("\"requestId\":\""), "{log}");
    assert!(log.contains("\"durationMs\":"), "{log}");
    assert!(log.contains("\"path\":\"/api/v1/health/live\""), "{log}");
    assert!(log.contains("\"status\":200"), "{log}");
    assert!(log.contains("\"errorCode\":\"-\""), "{log}");
    assert!(
        log.contains("\"path\":\"/api/unknown\"") && log.contains("\"errorCode\":\"NOT_FOUND\""),
        "错误响应日志应带统一错误码且不含查询串：{log}"
    );
    assert!(!log.contains("SECOND-CANARY"), "{log}");
    assert!(
        log.contains("\"keySource\":\"环境变量 EM_TEST_TRIPO_KEY\""),
        "已配置状态应记录密钥来源（不含密钥）：{log}"
    );
}

// ---------------------------------------------------------------------------
// backup / restore（T20 起为真实实现；本文件只做 CLI 层面的冒烟）
// ---------------------------------------------------------------------------

/// 事实更新（T20，2026-09-13）：backup/restore 从"退出码 7 = 未实现"转为真实实现，
/// 7 的含义更新为"备份/恢复完整性校验失败"（ADR-031）。本用例改为验证 CLI 冒烟：
/// init 后的空 data-dir 可备份、备份可恢复到新目录、退出码为 0 且产物存在。
/// 完整覆盖（停服要求、损坏 blob、非空目标、导出包、schema 门禁）见
/// `crates/server/tests/backup_restore.rs`。
#[test]
fn backup_and_restore_smoke_with_stable_exit_codes() {
    let dir = TestDir::new("backup");
    let _data = init_data_dir(&dir);
    let out_path = dir.join("backup-out");

    let backup = run(
        dir.path(),
        &[
            "backup",
            "--data-dir",
            "data",
            "--out",
            out_path.to_str().unwrap(),
        ],
    );
    assert_eq!(backup.status, 0, "{backup:?}");
    assert!(backup.stdout.contains("备份完成"), "{backup:?}");
    assert!(out_path.join("manifest.json").is_file(), "{backup:?}");
    assert!(
        out_path.join("database/manual.sqlite3").is_file(),
        "备份必须包含一致快照：{backup:?}"
    );

    // 已存在的输出路径 → 不覆盖（退出码 4）。
    let again = run(
        dir.path(),
        &[
            "backup",
            "--data-dir",
            "data",
            "--out",
            out_path.to_str().unwrap(),
        ],
    );
    assert_eq!(again.status, 4, "{again:?}");
    assert!(again.stderr.contains("已存在"), "{again:?}");

    let restored = run(
        dir.path(),
        &[
            "restore",
            "--from",
            "backup-out",
            "--data-dir",
            "restored-data",
        ],
    );
    assert_eq!(restored.status, 0, "{restored:?}");
    assert!(restored.stdout.contains("恢复完成"), "{restored:?}");
    assert!(dir.join("restored-data/manual.sqlite3").is_file());

    // 目标已存在且非空 → 拒绝（退出码 4）。
    write_text(&dir.join("restored-data/tmp/keep.txt"), "keep");
    let refused = run(
        dir.path(),
        &[
            "restore",
            "--from",
            "backup-out",
            "--data-dir",
            "restored-data",
        ],
    );
    assert_eq!(refused.status, 4, "{refused:?}");
    assert!(refused.stderr.contains("非空"), "{refused:?}");

    // 损坏的备份（manifest 缺失）→ 退出码 7（完整性校验失败）。
    std::fs::remove_file(out_path.join("manifest.json")).unwrap();
    let corrupt = run(
        dir.path(),
        &[
            "restore",
            "--from",
            "backup-out",
            "--data-dir",
            "restored-again",
        ],
    );
    assert_eq!(corrupt.status, 7, "{corrupt:?}");
    assert!(
        !dir.join("restored-again").exists(),
        "校验失败不得创建目标目录"
    );
}

// ---------------------------------------------------------------------------
// 配置示例文件
// ---------------------------------------------------------------------------

#[test]
fn example_config_is_accepted_and_no_key_material_inside() {
    let dir = TestDir::new("example-config");
    let _data = init_data_dir(&dir);
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.example.toml");
    let text = std::fs::read_to_string(&example).expect("示例配置应存在");

    // 使用示例配置（显式 --config）执行 check：必须被接受（无未知键）。
    let out = run(
        dir.path(),
        &[
            "check",
            "--data-dir",
            "data",
            "--config",
            example.to_str().unwrap(),
        ],
    );
    // 示例中的 data_dir 指向 ./manual-data，但 CLI --data-dir 优先，仍指向已初始化的 data。
    assert_eq!(out.status, 0, "{out:?}");

    // 示例不得包含真实密钥形态的字符串（只允许环境变量名与占位说明）。
    for marker in ["sk-", "Bearer ", "api_key =", "password"] {
        assert!(
            !text.contains(marker),
            "示例配置不得包含密钥/密码字面量（发现 {marker}）"
        );
    }
}
