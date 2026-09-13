//! `cargo xtask contracts` / `contracts --check`
//!
//! 生成链（ADR-009）：Rust DTO → utoipa → contracts/openapi.json → openapi-typescript
//! → apps/web/src/api/generated.ts。前端禁止手抄 API 类型。
//!
//! `--check` 在临时目录重新生成再与工作树逐字节比较：有差异非零退出，且不修改工作树。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::util::{repo_root, run_in};

const GENERATOR_RELATIVE: &str = "node_modules/.bin/openapi-typescript";

/// `check = true` 时只比较不写入。
pub fn run(check: bool) -> Result<()> {
    let root = repo_root();
    let web_dir = root.join("apps/web");
    let generator = web_dir.join(GENERATOR_RELATIVE);
    if !generator.is_file() {
        bail!(
            "找不到 {}。生成前端类型前请先运行 `npm --prefix apps/web ci`。",
            generator.display()
        );
    }

    let openapi_json = everything_manual::http::openapi::openapi_pretty_json()
        .context("从 Rust DTO 导出 OpenAPI 失败")?;

    let committed_openapi = root.join("contracts/openapi.json");
    let committed_ts = web_dir.join("src/api/generated.ts");

    if check {
        let tmp_dir = temp_check_dir();
        std::fs::create_dir_all(&tmp_dir)
            .with_context(|| format!("创建临时目录失败：{}", tmp_dir.display()))?;
        let tmp_openapi = tmp_dir.join("openapi.json");
        let tmp_ts = tmp_dir.join("generated.ts");
        std::fs::write(&tmp_openapi, &openapi_json)?;
        generate_typescript(&generator, &web_dir, &tmp_openapi, &tmp_ts)?;

        let mut stale = Vec::new();
        compare_files(
            &committed_openapi,
            &tmp_openapi,
            "contracts/openapi.json",
            &mut stale,
        );
        compare_files(
            &committed_ts,
            &tmp_ts,
            "apps/web/src/api/generated.ts",
            &mut stale,
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);

        if !stale.is_empty() {
            bail!(
                "合同漂移：{} 与 Rust DTO 生成结果不一致。请运行 `cargo xtask contracts` 并提交生成物。",
                stale.join("、")
            );
        }
        println!("合同检查通过：openapi.json 与 generated.ts 均与 Rust DTO 一致。");
    } else {
        if let Some(parent) = committed_openapi.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&committed_openapi, &openapi_json)
            .with_context(|| format!("写入失败：{}", committed_openapi.display()))?;
        println!("已写入 {}", committed_openapi.display());

        generate_typescript(&generator, &web_dir, &committed_openapi, &committed_ts)?;
        println!("已生成 {}", committed_ts.display());
    }

    Ok(())
}

fn generate_typescript(
    generator: &Path,
    web_dir: &Path,
    input: &Path,
    output: &Path,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    run_in(
        web_dir,
        generator,
        [input.as_os_str(), "-o".as_ref(), output.as_os_str()],
    )
    .with_context(|| "openapi-typescript 生成失败")
}

fn compare_files(committed: &Path, generated: &Path, label: &str, stale: &mut Vec<String>) {
    let committed_bytes = std::fs::read(committed).unwrap_or_default();
    let generated_bytes = std::fs::read(generated).unwrap_or_default();
    if committed_bytes != generated_bytes {
        println!("  [漂移] {label}");
        stale.push(label.to_owned());
    } else {
        println!("  [一致] {label}");
    }
}

fn temp_check_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "everything-manual-contracts-check-{}",
        std::process::id()
    ))
}
