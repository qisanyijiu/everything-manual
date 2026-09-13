//! 一次性生成器（薄封装）：把 `test_support::generate` 的原创样例资产写入
//! `tests/fixtures/assets/`，写入前先结构自检、写入后打印 sha256。
//!
//! 生成逻辑在库里（`src/generate.rs`），因此 `fixture_harness.rs` 可以在测试进程内
//! 重新生成并断言"重新生成的字节 == 仓库中提交的字节"。
//!
//! 用法（仓库根）：
//! ```text
//! cargo run -p test-support --bin generate-fixtures            # 写入 tests/fixtures/assets/
//! cargo run -p test-support --bin generate-fixtures -- <dir>   # 写入指定目录
//! ```

use std::path::PathBuf;

use test_support::assets::{sha256_hex, validate_glb, validate_jpeg, validate_pdf, validate_png};

fn main() {
    let out_dir = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => test_support::fixtures_root().join("assets"),
    };
    std::fs::create_dir_all(&out_dir).expect("创建输出目录");

    for (name, bytes) in test_support::generate::build_all() {
        let report = match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("glb") => format!("{:?}", validate_glb(&bytes).expect("GLB 自检")),
            Some("pdf") => format!("{:?}", validate_pdf(&bytes).expect("PDF 自检")),
            Some("png") => format!("{:?}", validate_png(&bytes).expect("PNG 自检")),
            Some("jpg") => format!("{:?}", validate_jpeg(&bytes).expect("JPEG 自检")),
            _ => "（未知类型，跳过自检）".to_owned(),
        };
        std::fs::write(out_dir.join(name), &bytes).expect("写入样例资产");
        println!(
            "{name}\n  sha256 = {}\n  大小 = {} 字节\n  自检 = {report}",
            sha256_hex(&bytes),
            bytes.len()
        );
    }
}
