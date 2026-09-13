//! 价格目录（`price_catalog_path`；T11 / REQ-020、REQ-023）。
//!
//! **为什么是文件而不是代码常量**：价格与模型预设会变化，运营者需要在不改二进制、
//! 不重新发布的前提下更新；同时报价必须绑定"价格版本 + 快照日期"以便审计与复算
//! （contracts.md §4）。目录内容全部是**十进制字面量**，由
//! [`manual_core::cost::parse_decimal_scaled`] 精确换算为整数最小单位（`creditMinor`
//! / `usdMicros`），**禁止浮点**。
//!
//! 目录形状（示例见仓库根 `price-catalog.example.toml`）：
//!
//! ```toml
//! version = "2026-09-11"
//! snapshot_date = "2026-09-11"
//!
//! [[tripo.presets]]
//! preset = "tripo-h-v3.1-standard"
//! model = "v3.1-20260211"
//! credits = "30"                     # 每次生成 30 credits（十进制字面量）
//! # 其余字段省略时取架构 §5.3 的默认参数
//!
//! [manual_ai.models.gpt-5-mini]      # 键 = providers.manual_ai.model
//! input_usd_per_million_tokens = "0.25"
//! output_usd_per_million_tokens = "2.00"
//! image_usd_per_image = "0.01"
//! ```
//!
//! 规则：
//! - **未知键报错**（`deny_unknown_fields`，与配置文件的处理一致）；
//! - 无法解析的金额/负数/重复预设 = 目录错误 → `serve`/`check` 启动即失败
//!   （退出码 3），不静默跳过（"缺价格配置时不能宣称精确费用"，contracts.md §4）；
//! - 目录里没有的预设/模型 = 不支持生成（estimate 返回 422 `modelPresetUnsupported`
//!   或 409 `PRICE_CATALOG_MISSING`，见 `generation::estimate`）。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use manual_core::cost::{CREDIT_MINOR_SCALE, MoneyError, USD_MICROS_SCALE, price_to_minor};
use manual_core::generation::{ManualAiPricing, TripoParameters, TripoPricing};

/// 价格目录加载/解析错误（面向运维的可读文案；不产生任何近似值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    pub message: String,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CatalogError {}

fn error(message: impl Into<String>) -> CatalogError {
    CatalogError {
        message: message.into(),
    }
}

/// 解析后的价格目录（内存表示；进程启动时加载一次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceCatalog {
    /// 价格版本（引用在报价/快照/账本上的稳定标识）。
    pub version: String,
    /// 价格快照日期（展示用，ISO `YYYY-MM-DD`）。
    pub snapshot_date: String,
    /// 受支持的 Tripo 预设（顺序按目录声明，便于展示）。
    pub tripo_presets: Vec<TripoPreset>,
    /// 说明书 AI 模型定价（键 = `providers.manual_ai.model`）。
    pub manual_ai_models: Vec<ManualAiPricing>,
}

/// 一个受支持的 Tripo 多视图预设。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripoPreset {
    /// 预设名（estimate 请求的 `modelPreset`）。
    pub key: String,
    /// 供应商模型名与生成参数（冻结到报价与快照）。
    pub parameters: TripoParameters,
    /// 原始十进制字面量（展示）。
    pub credits_decimal: String,
    /// 每次生成的 creditMinor（1/100 credit，精确换算）。
    pub credit_minor: i64,
}

impl TripoPreset {
    /// 换算用的定价视图（报价金额从这里来；不含密钥）。
    pub fn pricing(&self) -> TripoPricing {
        TripoPricing {
            preset: self.key.clone(),
            model: self.parameters.model.clone(),
            credits_decimal: self.credits_decimal.clone(),
            credit_minor: self.credit_minor,
        }
    }
}

impl PriceCatalog {
    /// 按预设名查找。
    pub fn tripo_preset(&self, key: &str) -> Option<&TripoPreset> {
        self.tripo_presets.iter().find(|preset| preset.key == key)
    }

    /// 按模型名查找说明书 AI 定价。
    pub fn manual_ai_pricing(&self, model: &str) -> Option<&ManualAiPricing> {
        self.manual_ai_models
            .iter()
            .find(|pricing| pricing.model == model)
    }

    /// 支持的预设名清单（错误明细与 OpenAPI 描述共用）。
    pub fn preset_keys(&self) -> Vec<String> {
        self.tripo_presets
            .iter()
            .map(|preset| preset.key.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// TOML 形状
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    version: String,
    snapshot_date: String,
    tripo: TripoSection,
    /// 可以缺席（则说明书 AI 无可用单价，estimate 返回 409 并说明缺项）。
    #[serde(default)]
    manual_ai: ManualAiSection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TripoSection {
    presets: Vec<TripoPresetSection>,
}

/// Tripo 预设；省略的参数取架构 §5.3 的标准默认值（不静默改质量：默认值就是标准质量）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TripoPresetSection {
    preset: String,
    model: String,
    credits: String,
    #[serde(default = "default_true")]
    texture: bool,
    #[serde(default = "default_true")]
    pbr: bool,
    #[serde(default = "default_standard")]
    texture_quality: String,
    #[serde(default = "default_standard")]
    geometry_quality: String,
    #[serde(default = "default_face_limit")]
    face_limit: i64,
    #[serde(default)]
    quad: bool,
    #[serde(default)]
    generate_parts: bool,
}

fn default_true() -> bool {
    true
}

fn default_standard() -> String {
    "standard".to_owned()
}

fn default_face_limit() -> i64 {
    100_000
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualAiSection {
    /// 键 = 模型名（TOML `[manual_ai.models."gpt-5-mini"]`）。
    #[serde(default)]
    models: BTreeMap<String, ManualAiModelSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualAiModelSection {
    input_usd_per_million_tokens: String,
    output_usd_per_million_tokens: String,
    image_usd_per_image: String,
}

// ---------------------------------------------------------------------------
// 加载与校验
// ---------------------------------------------------------------------------

/// 从路径加载（文件必须存在；调用方已保证路径可用）。
pub fn load(path: &Path) -> Result<PriceCatalog, CatalogError> {
    let text = std::fs::read_to_string(path)
        .map_err(|io_error| error(format!("价格目录不可读 {}：{io_error}", path.display())))?;
    parse(&text)
}

/// 解析并按目录规则校验（金额精确换算、预设唯一、参数合法）。
pub fn parse(text: &str) -> Result<PriceCatalog, CatalogError> {
    let file: CatalogFile = toml::from_str(text)
        .map_err(|toml_error| error(format!("价格目录不是合法 TOML 或含未知键：{toml_error}")))?;

    if file.version.trim().is_empty() {
        return Err(error("价格目录缺少 version（价格版本是报价绑定的一部分）"));
    }
    if !is_iso_date(file.snapshot_date.trim()) {
        return Err(error(format!(
            "价格目录的 snapshot_date 必须是 YYYY-MM-DD：{}",
            file.snapshot_date
        )));
    }
    if file.tripo.presets.is_empty() {
        return Err(error("价格目录至少需要一个 [[tripo.presets]] 条目"));
    }

    let mut presets: Vec<TripoPreset> = Vec::new();
    for section in file.tripo.presets {
        let key = section.preset.trim().to_owned();
        if key.is_empty() {
            return Err(error("tripo 预设缺少 preset 名称"));
        }
        if presets.iter().any(|existing| existing.key == key) {
            return Err(error(format!("tripo 预设重复：{key}")));
        }
        if section.model.trim().is_empty() {
            return Err(error(format!("tripo 预设 {key} 缺少 model")));
        }
        if section.face_limit <= 0 {
            return Err(error(format!(
                "tripo 预设 {key} 的 face_limit 必须为正（{}）",
                section.face_limit
            )));
        }
        if section.texture_quality.trim().is_empty() || section.geometry_quality.trim().is_empty() {
            return Err(error(format!("tripo 预设 {key} 的质量参数不能为空")));
        }
        let credits_decimal = section.credits.trim().to_owned();
        let credit_minor =
            price_to_minor(&credits_decimal, CREDIT_MINOR_SCALE).map_err(|money_error| {
                error(format!(
                    "tripo 预设 {key} 的 credits 无法精确换算：{money_error}"
                ))
            })?;
        presets.push(TripoPreset {
            key,
            parameters: TripoParameters {
                model: section.model.trim().to_owned(),
                texture: section.texture,
                pbr: section.pbr,
                texture_quality: section.texture_quality.trim().to_owned(),
                geometry_quality: section.geometry_quality.trim().to_owned(),
                face_limit: section.face_limit,
                quad: section.quad,
                generate_parts: section.generate_parts,
            },
            credits_decimal,
            credit_minor,
        });
    }

    let mut manual_ai_models: Vec<ManualAiPricing> = Vec::new();
    for (model, section) in file.manual_ai.models {
        let model = model.trim().to_owned();
        if model.is_empty() {
            return Err(error("manual_ai 模型名为空"));
        }
        let input = money(
            &format!("manual_ai.models.{model}.input_usd_per_million_tokens"),
            &section.input_usd_per_million_tokens,
        )?;
        let output = money(
            &format!("manual_ai.models.{model}.output_usd_per_million_tokens"),
            &section.output_usd_per_million_tokens,
        )?;
        let image = money(
            &format!("manual_ai.models.{model}.image_usd_per_image"),
            &section.image_usd_per_image,
        )?;
        manual_ai_models.push(ManualAiPricing {
            model,
            input_usd_micros_per_million_tokens: input.1,
            output_usd_micros_per_million_tokens: output.1,
            image_usd_micros_per_image: image.1,
            input_price_decimal: input.0,
            output_price_decimal: output.0,
            image_price_decimal: image.0,
        });
    }

    Ok(PriceCatalog {
        version: file.version.trim().to_owned(),
        snapshot_date: file.snapshot_date.trim().to_owned(),
        tripo_presets: presets,
        manual_ai_models,
    })
}

/// 十进制字面量 → usdMicros（返回规范化后的字面量与整数金额）。
fn money(key: &str, literal: &str) -> Result<(String, i64), CatalogError> {
    let trimmed = literal.trim().to_owned();
    let micros = price_to_minor(&trimmed, USD_MICROS_SCALE)
        .map_err(|money_error: MoneyError| error(format!("{key} 无法精确换算：{money_error}")))?;
    Ok((trimmed, micros))
}

/// 宽松的 `YYYY-MM-DD` 形状检查（不引入日历运算；错误日期由运营者负责）。
fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库根的示例文件必须能被加载（防止"示例与解析器漂移"）。
    #[test]
    fn example_catalog_parses_and_matches_architecture_defaults() {
        let text = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../price-catalog.example.toml"
        ));
        let catalog = parse(text).expect("示例价格目录必须可加载");
        assert_eq!(catalog.version, "2026-09-11");
        let preset = catalog
            .tripo_preset("tripo-h-v3.1-standard")
            .expect("示例预设必须在场");
        assert_eq!(preset.credit_minor, 3000, "30 credits = 3000 creditMinor");
        assert_eq!(preset.parameters.model, "v3.1-20260211");
        assert!(preset.parameters.texture && preset.parameters.pbr);
        assert_eq!(preset.parameters.face_limit, 100_000);
        assert!(!preset.parameters.quad && !preset.parameters.generate_parts);
        assert!(!catalog.manual_ai_models.is_empty());
        assert!(catalog.manual_ai_pricing("gpt-5-mini").is_some());
    }

    #[test]
    fn catalog_rejects_unknown_keys_and_bad_money() {
        let unknown = r#"
version = "v1"
snapshot_date = "2026-09-11"
[[tripo.presets]]
preset = "p"
model = "m"
credits = "30"
credits_typo = "1"
[manual_ai.models.m1]
input_usd_per_million_tokens = "1"
output_usd_per_million_tokens = "1"
image_usd_per_image = "0"
"#;
        assert!(parse(unknown).is_err(), "未知键必须报错");

        let bad_money = r#"
version = "v1"
snapshot_date = "2026-09-11"
[[tripo.presets]]
preset = "p"
model = "m"
credits = "-1"
"#;
        assert!(parse(bad_money).is_err(), "负数金额必须报错");

        let bad_date = r#"
version = "v1"
snapshot_date = "2026/09/11"
[[tripo.presets]]
preset = "p"
model = "m"
credits = "30"
"#;
        assert!(parse(bad_date).is_err(), "快照日期形状必须可读");

        let duplicate = r#"
version = "v1"
snapshot_date = "2026-09-11"
[[tripo.presets]]
preset = "p"
model = "m"
credits = "30"
[[tripo.presets]]
preset = "p"
model = "m2"
credits = "31"
"#;
        assert!(parse(duplicate).is_err(), "重复预设必须报错");

        let no_presets = r#"
version = "v1"
snapshot_date = "2026-09-11"
[tripo]
presets = []
"#;
        assert!(parse(no_presets).is_err(), "空预设清单必须报错");
    }

    #[test]
    fn catalog_defaults_follow_architecture_section_5_3() {
        let minimal = r#"
version = "v1"
snapshot_date = "2026-09-11"
[[tripo.presets]]
preset = "p"
model = "m"
credits = "30"
"#;
        let catalog = parse(minimal).unwrap();
        let parameters = &catalog.tripo_presets[0].parameters;
        assert!(parameters.texture && parameters.pbr);
        assert_eq!(parameters.texture_quality, "standard");
        assert_eq!(parameters.geometry_quality, "standard");
        assert_eq!(parameters.face_limit, 100_000);
        assert!(!parameters.quad && !parameters.generate_parts);
        assert!(catalog.manual_ai_models.is_empty());
    }
}
