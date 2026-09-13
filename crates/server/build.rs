//! 构建期依赖声明与内嵌资源（embedded-ui）检查。
//!
//! 本脚本只做两件事：声明 rerun-if-changed、在启用 embedded-ui 时校验
//! `apps/web/dist` 存在。它不运行 npm、不联网、不生成任何文件（validation-release.md §2）。

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // 迁移随二进制内嵌（`sqlx::migrate!`，ADR-009）：SQL 变更必须触发重编译，
    // 否则二进制会继续内嵌旧 schema。目录项增减看目录 mtime，文件内容修改看逐文件。
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    println!("cargo:rerun-if-changed={}", migrations.display());
    for entry in walk_files(&migrations) {
        println!("cargo:rerun-if-changed={}", entry.display());
    }

    // build script 通过 CARGO_FEATURE_<FEATURE> 感知启用的 feature。
    if std::env::var_os("CARGO_FEATURE_EMBEDDED_UI").is_none() {
        return;
    }

    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist");
    let index = dist.join("index.html");
    if !index.is_file() {
        panic!(
            "embedded-ui 需要前端构建产物，但 {} 不存在。\
             请先运行 `cargo xtask dist`（或 `npm --prefix apps/web ci && npm --prefix apps/web run build`）。\
             该 feature 不允许在缺少前端产物时产出空壳二进制。",
            dist.display()
        );
    }

    // dist 内容变化必须触发重编译，避免改了前端却仍内嵌旧资源。
    println!("cargo:rerun-if-changed={}", index.display());
    for entry in walk_files(&dist) {
        println!("cargo:rerun-if-changed={}", entry.display());
    }
}

/// 递归列出目录下的文件（只读；目录不存在时返回空列表）。
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}
