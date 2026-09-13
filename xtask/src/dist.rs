//! `cargo xtask dist --target <triple>` —— 发布路径构建（validation-release.md §2、§6）。
//!
//! 流程：校验工具链 → `npm ci/typecheck/test/build` → `cargo build --release --locked
//! --features embedded-ui --target <triple>` → 产出 `dist/<triple>/`：
//!
//! ```text
//! everything-manual        单文件可执行程序（内嵌 web 静态资源、迁移、bundled SQLite）
//! SHA256SUMS               二进制的 sha256（标准 shasum -a 256 格式）
//! licenses.json            第三方许可证清单（Rust 依赖按 normal 边 + 目标平台过滤；
//!                          前端按 package-lock production 条目；含汇总与未知项）
//! build-info.json          版本/target/features/工具链/git/签名声明/隔离扫描/动态依赖
//! dynamic-dependencies.txt 原生构建时的 `otool -L` / `file` + `ldd` 原始输出（跨构建写明未采集）
//! ```
//!
//! 隔离证据（§7 第 7 步、任务卡"不得把构建环境带进运行包"）：扫描二进制中是否残留
//! 仓库根／`apps/web/dist`／用户主目录的绝对路径，发现即失败；同时断言输出目录不含
//! 二进制与上述清单以外的文件（防止 node_modules、源码或上一次构建的残留混入）。
//!
//! 可复现性：`--check-reproducible` 会在产出后 `cargo clean -p everything-manual
//! --release --target <triple>` 再重新构建一次并比对 sha256（独立重链接，不是"没有
//! 改动所以哈希相同"的空转证据）。
//!
//! 签名／公证：本命令不做（需要用户账号授权），仅在 build-info.json 与报告中声明未做。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::util::{
    capture_in, capture_in_with_stderr, host_triple, repo_root, run_in, run_in_env, sha256_hex,
};

const BINARY_NAME: &str = "everything-manual";
const FEATURE_SET: [&str; 1] = ["embedded-ui"];
/// 输出目录允许出现的文件（除此之外一律视为混入构建环境残留）。
const ALLOWED_FILES: [&str; 5] = [
    BINARY_NAME,
    "SHA256SUMS",
    "licenses.json",
    "build-info.json",
    "dynamic-dependencies.txt",
];

pub struct DistOptions {
    pub target: String,
    /// 产出后清掉本包构件再构建一次，比对二进制 sha256（可复现性证据）。
    pub check_reproducible: bool,
}

pub fn run(options: &DistOptions) -> Result<()> {
    let root = repo_root();
    let web_dir = root.join("apps/web");
    let target = options.target.as_str();

    ensure_target_installed(&root, target)?;
    let host = host_triple()?;
    let cross_compiled = host != target;
    let rustc_version = first_line(&capture_in(&root, "rustc", ["-vV"])?);
    let cargo_version = first_line(&capture_in(&root, "cargo", ["-V"])?);
    let node_version = capture_in(&root, "node", ["--version"])
        .context("构建前端需要 Node（运行期不需要）")?
        .trim()
        .to_owned();
    let npm_version = capture_in(&root, "npm", ["--version"])
        .context("构建前端需要 npm")?
        .trim()
        .to_owned();

    // 前端：严格按锁文件安装，typecheck 与测试不能用 build 代替。
    run_in(&web_dir, "npm", ["ci"])?;
    run_in(&web_dir, "npm", ["run", "typecheck"])?;
    run_in(&web_dir, "npm", ["run", "test", "--", "--run"])?;
    run_in(&web_dir, "npm", ["run", "build"])?;

    // Rust release：--locked 保证锁文件生效；embedded-ui 要求前面的 dist 已产出。
    build_release(&root, target)?;

    let source_binary = root
        .join("target")
        .join(target)
        .join("release")
        .join(BINARY_NAME);
    if !source_binary.is_file() {
        bail!(
            "release 构建完成但未找到二进制：{}",
            source_binary.display()
        );
    }

    let out_dir = root.join("dist").join(target);
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("创建输出目录失败：{}", out_dir.display()))?;
    let out_binary = out_dir.join(BINARY_NAME);
    std::fs::copy(&source_binary, &out_binary)
        .with_context(|| format!("拷贝二进制失败：{}", out_binary.display()))?;

    let sha256 = sha256_hex(&out_binary)?;
    let size = std::fs::metadata(&out_binary)?.len();

    std::fs::write(
        out_dir.join("SHA256SUMS"),
        format!("{sha256}  {BINARY_NAME}\n"),
    )?;
    write_licenses(&root, target, &out_dir.join("licenses.json"))?;
    let isolation = scan_binary(&out_binary, &root)?;
    let dependencies = collect_dynamic_dependencies(
        &out_binary,
        target,
        &host,
        &out_dir.join("dynamic-dependencies.txt"),
    )?;
    write_build_info(
        &root,
        target,
        &host,
        cross_compiled,
        &sha256,
        size,
        &rustc_version,
        &cargo_version,
        &node_version,
        &npm_version,
        &isolation,
        &dependencies,
        &out_dir.join("build-info.json"),
    )?;
    assert_clean_output_dir(&out_dir)?;

    let mut reproducible = None;
    if options.check_reproducible {
        println!("\n[可复现] 清理本包构件并重新构建（独立重链接）…");
        run_in(
            &root,
            "cargo",
            [
                "clean",
                "--package",
                "everything-manual",
                "--release",
                "--target",
                target,
            ],
        )?;
        build_release(&root, target)?;
        let rebuilt = sha256_hex(&source_binary)?;
        if rebuilt != sha256 {
            bail!(
                "可复现性检查失败：两次独立构建的二进制 sha256 不同\n  第一次 {sha256}\n  第二次 {rebuilt}"
            );
        }
        println!("[可复现] 两次独立构建 sha256 一致：{sha256}");
        reproducible = Some(rebuilt);
    }

    println!("\ndist 完成：");
    println!("  binary : {}", out_binary.display());
    println!("  sha256 : {sha256}");
    println!("  size   : {size} bytes");
    println!(
        "  target : {target}（host {host}{}）",
        if cross_compiled {
            "，跨构建"
        } else {
            "，原生"
        }
    );
    println!("  文件   : SHA256SUMS, licenses.json, build-info.json, dynamic-dependencies.txt");
    if let Some(rebuilt) = reproducible {
        println!("  可复现 : 独立重建后 sha256 仍为 {rebuilt}");
    }
    Ok(())
}

fn build_release(root: &Path, target: &str) -> Result<()> {
    // 归一化编译期绝对路径：依赖 crate 的 panic 位置（`<home>/.cargo/registry/src/…`）
    // 默认会原样写进二进制，暴露构建机路径且使同源构建难以跨机器比对。
    // `--remap-path-prefix` 把它们折到 `/build/...`；本函数同时是"运行包不含构建机
    // 路径"的实现手段（隔离扫描会强制验证结果）。
    let mut rustflags = Vec::new();
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        rustflags.push(format!("--remap-path-prefix={home}=/build/home"));
    }
    rustflags.push(format!(
        "--remap-path-prefix={}=/build/repo",
        root.display()
    ));
    let rustflags = rustflags.join(" ");
    run_in_env(
        root,
        "cargo",
        [
            "build",
            "--release",
            "--locked",
            "--features",
            "embedded-ui",
            "--target",
            target,
            "--package",
            "everything-manual",
        ],
        &[("RUSTFLAGS", &rustflags)],
    )
}

fn ensure_target_installed(root: &Path, target: &str) -> Result<()> {
    let installed = capture_in(root, "rustup", ["target", "list", "--installed"])
        .context("需要 rustup 管理固定工具链；请先安装 rustup 工具链")?;
    if !installed.lines().any(|line| line.trim() == target) {
        bail!(
            "目标 {target} 未安装。请先运行 `rustup target add {target}`（工具链见 rust-toolchain.toml）"
        );
    }
    Ok(())
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_owned()
}

// ---------------------------------------------------------------------------
// 许可证清单
// ---------------------------------------------------------------------------

/// 汇总第三方许可证：Rust 依赖按发布同一 feature 集与目标平台的 **normal 边** 生成；
/// 前端按 `package-lock.json` 的 production 条目生成（含 PDF.js 等内嵌运行资源）。
fn write_licenses(root: &Path, target: &str, out: &Path) -> Result<()> {
    let rust_section = rust_licenses(root, target)?;
    let web_section = web_licenses(root)?;

    let document = serde_json::json!({
        "schemaVersion": 2,
        "generatedBy": format!("cargo xtask dist --target {target}"),
        "target": target,
        "rust": rust_section,
        "web": web_section,
        "notes": [
            "Rust 清单与发布构建同源：同一 Cargo.lock、同一 feature 集（embedded-ui）、\
             同一目标平台过滤、normal 依赖边（不含 dev/build 工具链依赖）。",
            "本文件是许可证**标识**清单；各包的完整许可证文本在 crates.io 源码包内\
             （cargo registry 缓存）与 node_modules/<包>/LICENSE*。",
            "运行资源中的 PDF.js 及其编解码依赖（OpenJPEG / JBIG2 / QCMS / Liberation / Foxit）\
             的许可证文本随二进制内嵌在 /vendor/pdfjs/*/LICENSE_*，运行时可读取。",
            "构建工具链（Node/npm/Playwright）是构建期依赖，不随发布包分发。",
            "项目自身许可证未在此声明（仓库 publish=false 私有项目，许可证选择属所有者决定）。"
        ],
    });
    std::fs::write(out, serde_json::to_string_pretty(&document)? + "\n")
        .with_context(|| format!("写入失败：{}", out.display()))?;
    Ok(())
}

fn rust_licenses(root: &Path, target: &str) -> Result<serde_json::Value> {
    // normal 边 = 真正进入发布二进制的依赖（dev/build 依赖不随包分发）。
    let tree = capture_in(
        root,
        "cargo",
        [
            "tree",
            "--package",
            "everything-manual",
            "--features",
            "embedded-ui",
            "--target",
            target,
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--locked",
        ],
    )
    .context("cargo tree（normal 边）失败：无法确定发布依赖集合")?;
    let mut wanted: BTreeSet<(String, String)> = BTreeSet::new();
    for line in tree.lines() {
        let line = line.trim().trim_end_matches(" (*)");
        let Some((name, rest)) = line.split_once(" v") else {
            continue;
        };
        let version = rest.split_whitespace().next().unwrap_or_default();
        if name.is_empty() || version.is_empty() {
            continue;
        }
        wanted.insert((name.to_owned(), version.to_owned()));
    }

    let raw = capture_in(
        root,
        "cargo",
        ["metadata", "--format-version", "1", "--locked"],
    )?;
    let metadata: serde_json::Value =
        serde_json::from_str(&raw).context("解析 cargo metadata 输出失败")?;
    let packages = metadata["packages"]
        .as_array()
        .context("cargo metadata 缺少 packages 数组")?;

    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut by_license: BTreeMap<String, u64> = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();
    for package in packages {
        let name = package["name"].as_str().unwrap_or_default().to_owned();
        let version = package["version"].as_str().unwrap_or_default().to_owned();
        if !wanted.contains(&(name.clone(), version.clone())) {
            continue;
        }
        let license = package["license"].as_str().map(str::to_owned);
        let license_file = package["license_file"].as_str().map(str::to_owned);
        match &license {
            Some(license) => *by_license.entry(license.clone()).or_default() += 1,
            None => unknown.push(format!("{name}@{version}")),
        }
        entries.push(serde_json::json!({
            "name": name,
            "version": version,
            "license": license,
            "licenseFile": license_file,
            "source": package["source"],
        }));
    }
    entries.sort_by(|left, right| {
        let left_key = (
            left["name"].as_str().unwrap_or_default(),
            left["version"].as_str().unwrap_or_default(),
        );
        let right_key = (
            right["name"].as_str().unwrap_or_default(),
            right["version"].as_str().unwrap_or_default(),
        );
        left_key.cmp(&right_key)
    });
    if entries.is_empty() {
        bail!("Rust 许可证清单为空：cargo tree / metadata 的依赖集合匹配失败");
    }

    Ok(serde_json::json!({
        "featureSet": FEATURE_SET,
        "edges": "normal",
        "packageCount": entries.len(),
        "byLicense": by_license,
        "unknown": unknown,
        "packages": entries,
    }))
}

fn web_licenses(root: &Path) -> Result<serde_json::Value> {
    let web_dir = root.join("apps/web");
    let lock_path = web_dir.join("package-lock.json");
    let raw = std::fs::read_to_string(&lock_path)
        .with_context(|| format!("读取 {} 失败（发布构建需先 npm ci）", lock_path.display()))?;
    let lock: serde_json::Value =
        serde_json::from_str(&raw).context("package-lock 不是合法 JSON")?;
    let packages = lock["packages"]
        .as_object()
        .context("package-lock 缺少 packages")?;

    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut by_license: BTreeMap<String, u64> = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();
    for (path, entry) in packages {
        if path.is_empty() || entry["dev"].as_bool() == Some(true) {
            continue;
        }
        let name = path
            .rsplit("node_modules/")
            .next()
            .unwrap_or(path)
            .to_owned();
        let version = entry["version"].as_str().unwrap_or_default().to_owned();
        // 已安装（本平台）时以 node_modules 的 package.json 为准，否则退回锁文件声明。
        let installed = web_dir.join(path).join("package.json");
        let license = std::fs::read_to_string(&installed)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|package| {
                package["license"]
                    .as_str()
                    .or_else(|| {
                        package["licenses"]
                            .as_array()
                            .filter(|licenses| !licenses.is_empty())
                            .map(|_| "见 licenses 数组")
                    })
                    .map(str::to_owned)
            })
            .or_else(|| entry["license"].as_str().map(str::to_owned));
        match &license {
            Some(license) => *by_license.entry(license.clone()).or_default() += 1,
            None => unknown.push(format!("{name}@{version}")),
        }
        entries.push(serde_json::json!({
            "name": name,
            "version": version,
            "license": license,
            "optional": entry["optional"].as_bool().unwrap_or(false),
            "installed": installed.is_file(),
        }));
    }
    entries.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });

    Ok(serde_json::json!({
        "source": "apps/web/package-lock.json（production 条目；已安装包读 node_modules/<包>/package.json）",
        "packageCount": entries.len(),
        "byLicense": by_license,
        "unknown": unknown,
        "packages": entries,
    }))
}

// ---------------------------------------------------------------------------
// 隔离扫描与动态依赖
// ---------------------------------------------------------------------------

/// 扫描二进制中是否残留构建机绝对路径（组合出"运行包不含构建环境"的证据）。
fn scan_binary(binary: &Path, root: &Path) -> Result<serde_json::Value> {
    let bytes =
        std::fs::read(binary).with_context(|| format!("读取二进制失败：{}", binary.display()))?;
    let mut forbidden: Vec<(String, usize)> = Vec::new();
    let mut patterns: Vec<String> = vec![
        root.display().to_string(),
        root.join("apps/web/dist").display().to_string(),
        root.join("apps/web/node_modules").display().to_string(),
    ];
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        patterns.push(home);
    }
    for pattern in &patterns {
        let count = count_occurrences(&bytes, pattern.as_bytes());
        if count > 0 {
            forbidden.push((pattern.clone(), count));
        }
    }
    let node_modules = count_occurrences(&bytes, b"node_modules");
    let remapped_repo = count_occurrences(&bytes, b"/build/repo");
    let remapped_home = count_occurrences(&bytes, b"/build/home");
    if !forbidden.is_empty() {
        bail!(
            "二进制含构建机绝对路径（违反单二进制隔离）：{forbidden:?}\n\
             常见原因：① 依赖 crate 的 panic 位置未归一化（应保持 build_release 的 \
             --remap-path-prefix 生效）；② 代码/构建脚本把 env!(\"CARGO_MANIFEST_DIR\") \
             之类的编译期绝对路径写进产物或内嵌资源。"
        );
    }
    if remapped_repo == 0 && remapped_home == 0 && node_modules > 0 {
        bail!(
            "二进制既无归一化路径也无构建机路径，但出现了 node_modules 字面量：\
             请人工复核是否把前端构建目录打进了产物"
        );
    }
    Ok(serde_json::json!({
        "scannedBytes": bytes.len(),
        "forbiddenPatterns": patterns,
        "hits": 0,
        "remappedRepoPathHits": remapped_repo,
        "remappedHomePathHits": remapped_home,
        "nodeModulesLiteralHits": node_modules,
        "note": "二进制中不含仓库根 / apps/web/dist / node_modules 目录 / 用户主目录的绝对路径；\
                 编译期路径已被 --remap-path-prefix 归一化为 /build/repo 与 /build/home\
                 （命中次数见 remapped*PathHits）。nodeModulesLiteralHits 只是文本串出现次数\
                 （压缩 JS 或依赖元数据里的字面量），不代表携带构建环境。",
    }))
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count = 0;
    let mut offset = 0;
    while offset + needle.len() <= haystack.len() {
        match haystack[offset..]
            .windows(needle.len())
            .position(|window| window == needle)
        {
            Some(position) => {
                count += 1;
                offset += position + needle.len();
            }
            None => break,
        }
    }
    count
}

/// 动态依赖采集的适用判据：构建机与目标平台**同架构、同 OS**。
///
/// 不要求 triple 完全相等。§6 禁止的是"拿交叉编译退出码 0 当运行证据"——反例是
/// 在 macOS 上构建 Linux 产物（系统库/内核语义完全不同）。而 `x86_64-unknown-linux-gnu`
/// 构建机上构建 `x86_64-unknown-linux-musl`（静态）时，构建机与目标平台同 OS 同架构，
/// `file`/`ldd`/`readelf -d` 的结论对目标产物有效，且该产物就在同一环境由 `smoke` 真实运行；
/// 差集只有 libc/ABI（gnu→musl），由 `staticLinked` 结论体现。
fn same_arch_and_os(a: &str, b: &str) -> bool {
    fn key(triple: &str) -> (String, String) {
        // <arch>[-<vendor>]-<os>[-<env>]：四段及以上时 os 在 env 之前（末段是 env）。
        let parts: Vec<&str> = triple.split('-').collect();
        let arch = parts.first().copied().unwrap_or_default().to_owned();
        let os = if parts.len() >= 4 {
            parts[parts.len() - 2]
        } else {
            parts.last().copied().unwrap_or_default()
        };
        (arch, os.to_owned())
    }
    key(a) == key(b)
}

/// 原生（或同架构同 OS）构建时采集系统动态依赖清单；否则写出"未采集"的说明。
fn collect_dynamic_dependencies(
    binary: &Path,
    target: &str,
    host: &str,
    out: &Path,
) -> Result<serde_json::Value> {
    let same_platform = same_arch_and_os(host, target);
    if !same_platform {
        std::fs::write(
            out,
            format!(
                "跨平台构建（host {host} → target {target}）：动态依赖未在本机采集。\n\
                 请在目标平台（同架构同 OS）重新构建后采集 `otool -L` / `file` + `ldd` 输出；\
                 §6 要求以目标平台原生运行为准。\n"
            ),
        )
        .with_context(|| format!("写入失败：{}", out.display()))?;
        return Ok(serde_json::json!({
            "collected": false,
            "reason": "cross-platform",
            "host": host,
            "target": target,
            "file": out.file_name().and_then(|name| name.to_str()),
        }));
    }

    let mut report = String::new();
    let mut static_linked = None;
    let mut only_system = None;
    if cfg!(target_os = "macos") {
        let (_, otool) = capture_in_with_stderr(Path::new("."), "otool", ["-L", &path_str(binary)])
            .context("otool -L 采集失败")?;
        // otool -L 的依赖行以制表符缩进，首个字段是库路径；只允许 macOS 系统库。
        let offenders: Vec<String> = otool
            .lines()
            .filter(|line| line.starts_with('\t') || line.starts_with("    "))
            .filter_map(|line| line.split_whitespace().next())
            .filter(|library| {
                !(library.starts_with("/usr/lib/") || library.starts_with("/System/Library/"))
            })
            .map(str::to_owned)
            .collect();
        if !offenders.is_empty() {
            bail!(
                "macOS 动态依赖含非系统库（§6 禁止 Homebrew／构建机路径）：{}",
                offenders.join("；")
            );
        }
        report.push_str("$ otool -L <binary>\n");
        report.push_str(&otool);
        report.push_str(
            "\n判定：仅链接 macOS 系统库（/usr/lib 与 /System/Library），\
             无 Homebrew／/usr/local／构建机路径。\n",
        );
        only_system = Some(true);
    } else if cfg!(target_os = "linux") {
        let (_, file_output) = capture_in_with_stderr(Path::new("."), "file", [path_str(binary)])
            .context("file 采集失败（容器内需安装 file）")?;
        let (_, ldd_output) = capture_in_with_stderr(Path::new("."), "ldd", [path_str(binary)])
            .unwrap_or((false, String::from("ldd 不可用")));
        let mut readelf_output = String::new();
        if let Ok((_, text)) =
            capture_in_with_stderr(Path::new("."), "readelf", ["-d", &path_str(binary)])
        {
            readelf_output = text;
        }
        let is_static =
            file_output.contains("statically linked") || file_output.contains("static-pie linked");
        static_linked = Some(is_static);
        report.push_str("$ file <binary>\n");
        report.push_str(file_output.trim_end());
        report.push_str("\n\n$ ldd <binary>\n");
        report.push_str(ldd_output.trim_end());
        if !readelf_output.is_empty() {
            // 全文（不截断）＋ DT_NEEDED 计数：静态 musl 二进制的结论可一眼核对。
            let needed: Vec<&str> = readelf_output
                .lines()
                .filter(|line| line.contains("(NEEDED)"))
                .collect();
            report.push_str("\n\n$ readelf -d <binary>\n");
            report.push_str(readelf_output.trim_end());
            report.push_str(&format!(
                "\n\nDT_NEEDED 条目数：{}（静态链接的 musl 单二进制应为 0）\n",
                needed.len()
            ));
            for line in needed {
                report.push_str(line);
                report.push('\n');
            }
        }
        report.push_str("\n\n判定：");
        if is_static {
            report.push_str(
                "静态链接（musl 单二进制），不含外部动态库依赖；\
                 运行时不需要额外安装 SQLite/Node/Python。\n",
            );
        } else {
            report.push_str("非静态链接：请核对 ldd 输出中的系统库清单与目标环境。\n");
        }
    } else {
        report.push_str("当前宿主平台未实现依赖采集（仅覆盖 macOS / Linux）。\n");
    }
    std::fs::write(out, report).with_context(|| format!("写入失败：{}", out.display()))?;
    Ok(serde_json::json!({
        "collected": true,
        "host": host,
        "target": target,
        "samePlatformAsBuild": target == host,
        "file": out.file_name().and_then(|name| name.to_str()),
        "staticLinked": static_linked,
        "onlySystemLibraries": only_system,
    }))
}

fn path_str(path: &Path) -> String {
    path.display().to_string()
}

/// 输出目录只能有约定的产物（防止 node_modules、源码或旧构建残留混入发布包）。
fn assert_clean_output_dir(out_dir: &Path) -> Result<()> {
    let mut unexpected: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(out_dir)
        .with_context(|| format!("读取输出目录失败：{}", out_dir.display()))?
        .filter_map(|entry| entry.ok())
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !ALLOWED_FILES.contains(&name.as_str()) {
            unexpected.push(name);
        }
    }
    if !unexpected.is_empty() {
        bail!(
            "输出目录含约定产物以外的文件（疑似构建环境残留）：{}；允许的文件：{ALLOWED_FILES:?}",
            unexpected.join("、")
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// build-info
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_build_info(
    root: &Path,
    target: &str,
    host: &str,
    cross_compiled: bool,
    sha256: &str,
    size: u64,
    rustc_version: &str,
    cargo_version: &str,
    node_version: &str,
    npm_version: &str,
    isolation: &serde_json::Value,
    dependencies: &serde_json::Value,
    out: &Path,
) -> Result<()> {
    let version = workspace_version(root)?;
    let git_commit = capture_in(root, "git", ["rev-parse", "HEAD"])
        .map(|value| value.trim().to_owned())
        .ok();
    let git_dirty = capture_in(root, "git", ["status", "--porcelain"])
        .map(|value| !value.trim().is_empty())
        .unwrap_or(true);
    let built_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .context("格式化构建时间失败")?;

    let document = serde_json::json!({
        "schemaVersion": 2,
        "product": BINARY_NAME,
        "version": version,
        "target": target,
        "host": host,
        "crossCompiled": cross_compiled,
        "profile": "release",
        "features": FEATURE_SET,
        "rustc": rustc_version,
        "cargo": cargo_version,
        "node": node_version,
        "npm": npm_version,
        "builtAt": built_at,
        "git": { "commit": git_commit, "dirty": git_dirty },
        "binary": { "file": BINARY_NAME, "sha256": sha256, "bytes": size },
        "isolation": isolation,
        "dynamicDependencies": dependencies,
        "signature": {
            "signed": false,
            "notarized": false,
            "note": "未做代码签名与公证：macOS 签名/公证需要用户账号授权（validation-release §6）。\
                     分发到其他机器时需在系统设置中按 Gatekeeper 提示放行，或由所有者用自有证书签名。",
        },
        "deployment": {
            "tls": "无内置 TLS 监听（MVP，PRD §5.6 / A-15）；配置 tls.* 时拒绝启动（fail-closed）",
            "boundary": "仅支持 loopback，或位于显式受信反向代理之后（代理负责 TLS 终止）",
            "runtimeRequirements": "操作系统系统库（macOS）/ 无额外动态库（Linux musl 静态）；\
                                    不需要 Node/Python/外部数据库/PDF 程序",
            "dataDir": "用户数据在独立 data-dir；单二进制不等于无数据目录（ADR-002）",
        },
    });
    std::fs::write(out, serde_json::to_string_pretty(&document)? + "\n")
        .with_context(|| format!("写入失败：{}", out.display()))?;
    Ok(())
}

fn workspace_version(root: &Path) -> Result<String> {
    let raw = capture_in(
        root,
        "cargo",
        ["metadata", "--format-version", "1", "--locked"],
    )?;
    let metadata: serde_json::Value = serde_json::from_str(&raw)?;
    let version = metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package["name"] == BINARY_NAME)
        })
        .and_then(|package| package["version"].as_str())
        .context("cargo metadata 中找不到 everything-manual 版本")?;
    Ok(version.to_owned())
}
