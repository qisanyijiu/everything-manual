//! 结构守卫（审查测试；ADR-034 的"结构性保障"部分）。
//!
//! 回答"如果明天新增一个供应商错误文本字段，它会不会漏？"：
//!
//! 1. **唯一 SQL 写入层**：供应商事实表（`job_stages` / `provider_attempts`）的
//!    INSERT/UPDATE 只允许出现在 `src/storage/repo/` 下——任何绕过仓储层的直写会让
//!    本测试失败（强制走脱敏入口所在的层）；
//! 2. **写入函数的脱敏义务**：这两个仓储文件里"带写入语句"的函数必须调用统一脱敏
//!    入口（`redact_*`），或在本文件的**显式豁免清单**中登记理由（身份标识符、纯
//!    状态更新、常量输入）。新增写入函数而没做脱敏、也没登记理由 → 本测试失败。
//!
//! 这只是静态防线；行为防线见 `redaction_persistence.rs`（落库入口）、
//! `backup_restore.rs`（快照/恢复）、QA 的 `qa_*.rs`（全库 canary 扫描）与
//! dist 级脚本（备份/DTO/日志三层）。
//!
//! 注意：本测试只读源码文本，不执行任何网络/数据库操作。

use std::path::{Path, PathBuf};

/// 供应商事实表（按合同只存系统/供应商事实的持久化表；ADR-032）。
const FACT_TABLES: [&str; 2] = ["job_stages", "provider_attempts"];

/// 仓储层目录（唯一允许写事实表的位置）。
const REPO_DIR_PREFIX: &str = "storage/repo";

/// 写入语句的模式（行内含其一即视为写语句）。
fn write_patterns() -> Vec<String> {
    let mut patterns = Vec::new();
    for table in FACT_TABLES {
        patterns.push(format!("INSERT INTO {table}"));
        patterns.push(format!("UPDATE {table}"));
        patterns.push(format!("DELETE FROM {table}"));
    }
    patterns
}

fn collect_rust_files(base: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(base).expect("读取源码目录") {
        let entry = entry.expect("目录项");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
}

/// 事实表的写语句只允许出现在仓储层。
#[test]
fn supplier_fact_table_writes_live_only_in_repo_layer() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("src");
    let mut files = Vec::new();
    collect_rust_files(&source, &mut files);
    assert!(!files.is_empty(), "未找到源码文件（路径基准错误？）");

    let patterns = write_patterns();
    let mut offenders = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("读取源码");
        for (number, line) in text.lines().enumerate() {
            // 注释行不算写语句（例如文档里引用表名）。
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue;
            }
            if patterns.iter().any(|pattern| line.contains(pattern)) {
                let relative = file
                    .strip_prefix(manifest)
                    .unwrap_or(file)
                    .display()
                    .to_string();
                if !relative.starts_with(&format!("src/{REPO_DIR_PREFIX}")) {
                    offenders.push(format!("{relative}:{}：{}", number + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "供应商事实表的写语句必须只出现在 src/{REPO_DIR_PREFIX}/（统一脱敏入口所在层）：\n{}",
        offenders.join("\n")
    );
}

/// 自由文本列的写入函数必须脱敏或登记豁免（原因 + 影响面见下方清单）。
///
/// 判定范围（**扫描整个 `src/storage/repo/`**，新增仓储文件/新表同样适用）：
/// - 任何写 `job_stages` / `provider_attempts` 的语句；或
/// - 任何写语句涉及 `last_error` / `needs_input_json` / `usage_json` 三种自由文本列。
///
/// 豁免清单（函数名 → 理由；改动这些函数时重新审视）：
/// - 身份标识符：`remote_task_id`/`response_id`/`lease_owner` 是对账/恢复用的 id
///   （按 task_id 重查链接的判据），不是文本；脱敏会破坏恢复语义（ADR-032 第 2 条）。
/// - 常量/纯状态：布尔状态、时间戳、epoch、计数与 `NULL` 清空不携带外部文本。
/// - 常量输入：`page_set_json` 来自本服务计算的页号列表（非供应商文本）。
#[test]
fn write_functions_either_redact_or_are_exempt() {
    const EXEMPT: [(&str, &str); 10] = [
        ("insert", "page_set 是本地计划页号；其余文本列写 NULL"),
        ("claim_next", "只写 lease_owner（worker 身份）与状态"),
        ("renew_lease", "只续租约时间"),
        (
            "reset_result_fact",
            "usage_json 置 NULL（清除引用，不写入文本）",
        ),
        ("cancel_unsubmitted_for_job", "只改状态与租约"),
        ("take_over_expired", "只改租约 owner/epoch"),
        ("create_intent", "request_hash 是本地计算的输入指纹"),
        ("mark_submitting", "只改 submit_state 与时间"),
        (
            "record_remote_task_id",
            "remote_task_id 是恢复判据（id，脱敏会破坏按 task_id 重查）",
        ),
        (
            "record_sync_response",
            "response_id 是 opaque id（不假定可重取）",
        ),
    ];
    /// 自由文本列：出现即要求该函数脱敏或豁免（任何仓储文件）。
    const SENSITIVE_COLUMNS: [&str; 3] = ["last_error", "needs_input_json", "usage_json"];

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_dir = manifest.join("src/storage/repo");
    let mut files = Vec::new();
    collect_rust_files(&repo_dir, &mut files);
    assert!(!files.is_empty(), "未找到仓储源码（路径基准错误？）");

    let patterns = write_patterns();
    let mut offenders = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("读取仓储源码");
        for (name, body) in top_level_functions(&text) {
            let writes_fact_table = patterns
                .iter()
                .any(|pattern| body.contains(pattern.as_str()));
            let has_write_verb = body.contains("INSERT INTO") || body.contains("UPDATE ");
            let touches_sensitive_column =
                has_write_verb && SENSITIVE_COLUMNS.iter().any(|column| body.contains(column));
            if !writes_fact_table && !touches_sensitive_column {
                continue;
            }
            let redacts = body.contains("redact_");
            let exempt = EXEMPT
                .iter()
                .any(|(exempt_name, _reason)| *exempt_name == name);
            if !redacts && !exempt {
                offenders.push(format!("{}::{name}", file.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "以下仓储写入函数既未调用统一脱敏入口、也不在豁免清单（新增字段/函数时请二选一）：\n{}",
        offenders.join("\n")
    );
}

/// 粗略切分顶层函数（`fn` / `pub fn` / `pub async fn` / `async fn` 起行）为
/// `(函数名, 函数体文本)`；本测试只用于仓储文件的静态审查。
fn top_level_functions(text: &str) -> Vec<(String, String)> {
    let mut functions: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let starts_function = trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub const fn ");
        if line.starts_with(' ') || !starts_function {
            if let Some((_name, body)) = functions.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        let name = trimmed
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .find(|token| !token.is_empty() && !matches!(*token, "pub" | "async" | "fn" | "const"))
            .unwrap_or("unknown")
            .to_owned();
        functions.push((name, format!("{line}\n")));
    }
    functions
}
