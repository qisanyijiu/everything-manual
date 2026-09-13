//! 费用换算与账本状态机（contracts.md §1/§4；T11）。
//!
//! 合同要点（本模块把它们变成可判定的纯函数）：
//! - **不使用浮点**：Tripo 用 `creditMinor`（1/100 credit），USD 用 `usdMicros`
//!   （1/1,000,000 USD）；供应商小数字面量用**精确 decimal 解析**再转换
//!   （[`parse_decimal_scaled`]，内部 i128 整数运算 + 显式舍入模式）；
//! - **保守上界**：报价的预算口径取 ceiling（向上取整），宁可多预留也不允许
//!   低估（[`Rounding::Ceil`]；预计口径见 `generation` 模块）；
//! - **预算语义**：预留/结算/释放是事务内幂等的状态转换
//!   （[`next_ledger_state`]）；结果未知（unknown）**保留预留、不得把实际费用填 0**。
//!
//! 本模块是纯逻辑：不依赖 SQLx／Axum／系统时钟，也不做 IO；持久化在
//! `crates/server/src/storage/repo/ledger.rs`。

use std::fmt;

use crate::domain::{Currency, LedgerState};

/// `creditMinor` 的小数位数（1/100 credit）。
pub const CREDIT_MINOR_SCALE: u32 = 2;
/// `usdMicros` 的小数位数（1/1,000,000 USD）。
pub const USD_MICROS_SCALE: u32 = 6;

/// 舍入模式（只对上界与预计的口径不同，不允许"无舍入的近似"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// 向下取整（截断）。
    Floor,
    /// 四舍五入（余数恰为一半时向上）。
    HalfUp,
    /// 向上取整：**保守上界与预留**使用该模式。
    Ceil,
}

/// 金额换算错误（不产生任何近似值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoneyError {
    /// 供应商十进制字面量无法解析（空、含符号/指数/非数字字符）。
    InvalidDecimal { literal: String, reason: String },
    /// 负数金额（价格与用量都必须非负）。
    Negative { literal: String },
    /// 溢出 i64 最小单位（拒绝静默回绕）。
    Overflow { detail: String },
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecimal { literal, reason } => {
                write!(f, "无法解析十进制字面量 {literal:?}：{reason}")
            }
            Self::Negative { literal } => write!(f, "金额不能为负：{literal:?}"),
            Self::Overflow { detail } => write!(f, "金额溢出：{detail}"),
        }
    }
}

impl std::error::Error for MoneyError {}

fn invalid(literal: &str, reason: impl Into<String>) -> MoneyError {
    MoneyError::InvalidDecimal {
        literal: literal.to_owned(),
        reason: reason.into(),
    }
}

/// 精确解析供应商十进制字面量并换算为最小整数单位。
///
/// 语义（contracts.md §1）：
/// - 只接受 `[整数部分][.小数部分]`（允许省略任一端的数字，"30."/".5" 合法），
///   不接受符号、科学计数法、千分位、空白以外的任何杂质（**不猜**）；
/// - 小数位数超过 `scale` 时按 `rounding` 处理余数（整数运算，无浮点）；
/// - 结果必须是 i64（creditMinor=1/100、usdMicros=1/1e6 的合法金额）。
///
/// ```text
/// parse_decimal_scaled("30",    2, Ceil)   == 3000    // 30.00 credits
/// parse_decimal_scaled("0.005", 2, Ceil)   == 1       // 0.005 → 0.01（保守）
/// parse_decimal_scaled("0.005", 2, HalfUp) == 1
/// parse_decimal_scaled("0.005", 2, Floor)  == 0
/// parse_decimal_scaled("0.0000005", 6, Ceil) == 1     // 0.5 micros → 1 micro
/// ```
pub fn parse_decimal_scaled(
    literal: &str,
    scale: u32,
    rounding: Rounding,
) -> Result<i64, MoneyError> {
    let trimmed = literal.trim();
    if trimmed.is_empty() {
        return Err(invalid(literal, "空字符串"));
    }
    if trimmed.starts_with('-') || trimmed.starts_with('+') {
        return Err(MoneyError::Negative {
            literal: literal.to_owned(),
        });
    }
    if let Some(exponent) = trimmed.find(['e', 'E']) {
        return Err(invalid(
            literal,
            format!("不支持科学计数法（位置 {exponent}）"),
        ));
    }
    let (int_part, frac_part) = match trimmed.split_once('.') {
        Some((int_part, frac_part)) => (int_part, frac_part),
        None => (trimmed, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(invalid(literal, "没有数字"));
    }
    if let Some(bad) = int_part
        .chars()
        .chain(frac_part.chars())
        .find(|ch| !ch.is_ascii_digit())
    {
        return Err(invalid(literal, format!("出现非数字字符 {bad:?}")));
    }

    let mut int_value: i128 = 0;
    for ch in int_part.chars() {
        int_value = int_value * 10 + i128::from(ch as u8 - b'0');
    }
    let scale_factor = 10_i128.pow(scale);
    int_value = int_value
        .checked_mul(scale_factor)
        .ok_or_else(|| MoneyError::Overflow {
            detail: format!("整数部分超出范围：{literal:?}"),
        })?;

    let mut frac_value: i128 = 0;
    for ch in frac_part.chars() {
        frac_value = frac_value * 10 + i128::from(ch as u8 - b'0');
    }
    let frac_digits = frac_part.chars().count() as u32;

    let frac_scaled = if frac_digits <= scale {
        frac_value
            .checked_mul(10_i128.pow(scale - frac_digits))
            .ok_or_else(|| MoneyError::Overflow {
                detail: format!("小数部分超出范围：{literal:?}"),
            })?
    } else {
        let divisor = 10_i128.pow(frac_digits - scale);
        let quotient = frac_value / divisor;
        let remainder = frac_value % divisor;
        match rounding {
            Rounding::Floor => quotient,
            Rounding::Ceil => {
                if remainder > 0 {
                    quotient + 1
                } else {
                    quotient
                }
            }
            Rounding::HalfUp => {
                if remainder * 2 >= divisor {
                    quotient + 1
                } else {
                    quotient
                }
            }
        }
    };

    let total = int_value + frac_scaled;
    i64::try_from(total).map_err(|_| MoneyError::Overflow {
        detail: format!("换算结果超出 i64 最小单位：{literal:?}（scale={scale}）"),
    })
}

/// `value * multiplier / divisor` 的向上取整（保守上界；全部整数运算）。
///
/// 用于"用量 × 单价"：例如 token 数 × 每 100 万 token 单价（`divisor = 1_000_000`）。
pub fn mul_div_ceil(value: i64, multiplier: i64, divisor: i64) -> Result<i64, MoneyError> {
    if value < 0 || multiplier < 0 {
        return Err(MoneyError::Negative {
            literal: format!("value={value} multiplier={multiplier}"),
        });
    }
    if divisor <= 0 {
        return Err(MoneyError::Overflow {
            detail: format!("除数必须为正：{divisor}"),
        });
    }
    let product = i128::from(value) * i128::from(multiplier);
    let divisor = i128::from(divisor);
    // 手写向上取整（`i128::div_ceil` 在当前工具链仍是 unstable，见 E0658 实测）。
    let quotient = (product + divisor - 1) / divisor;
    i64::try_from(quotient).map_err(|_| MoneyError::Overflow {
        detail: format!("{value} × {multiplier} / {divisor} 超出 i64"),
    })
}

/// 两个金额（最小单位）相加；溢出报错（不静默回绕）。
pub fn add_minor(a: i64, b: i64) -> Result<i64, MoneyError> {
    a.checked_add(b).ok_or_else(|| MoneyError::Overflow {
        detail: format!("{a} + {b} 超出 i64"),
    })
}

/// `credits`（1/100 credit）的可读十进制表示，固定两位小数：`3000 → "30.00"`。
pub fn format_credits(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let absolute = minor.unsigned_abs();
    format!("{sign}{}.{:02}", absolute / 100, (absolute % 100) as u32)
}

/// `usdMicros`（1/1e6 USD）的可读十进制表示：至少两位、最多六位小数，去掉多余尾零。
///
/// `300_000 → "0.30"`、`12_345 → "0.012345"`、`1 → "0.000001"`、`0 → "0.00"`。
pub fn format_usd_micros(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let absolute = micros.unsigned_abs();
    let integer = absolute / 1_000_000;
    let mut fraction = format!("{:06}", absolute % 1_000_000);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{sign}{integer}.{fraction}")
}

/// 价格目录里的十进制单价 → 最小整数单位（统一走 ceiling 保守口径）。
///
/// 与 [`parse_decimal_scaled`] 的唯一差别是固定 [`Rounding::Ceil`]：价格目录的换算
/// 永远不允许低估（宁可多预留）。
pub fn price_to_minor(literal: &str, scale: u32) -> Result<i64, MoneyError> {
    parse_decimal_scaled(literal, scale, Rounding::Ceil)
}

/// 账本事件（预留 → 结算/释放/未决）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerEvent {
    /// 按实际金额结算（只有拿到供应商计费事实才允许）。
    Settle { actual_minor: i64 },
    /// 释放预留：**只允许在明确未计费时**使用（例如可证明未被接受的失败）；
    /// 对 `unknown` 的释放必须来自管理员对账决定，不能由自动路径发起。
    Release,
    /// 结果未知：保留预留、`actual` 保持 NULL（不得填 0）。
    MarkUnknown,
}

/// 账本状态转换；非法转换返回 `None`（调用方不得静默忽略）。
///
/// 规则（contracts.md §4）：
/// - `reserved → settled`（按实际结算）、`reserved|unknown → released`（明确未计费/对账决定）、
///   `reserved → unknown`（结果未知，保留预留）；
/// - `unknown → settled`：对账后按实际结算；
/// - `settled`/`released` 是终态：重复事件由调用方按幂等处理（同值视为成功），
///   不同值属于冲突，不能改写已落账的事实。
pub const fn next_ledger_state(current: LedgerState, event: LedgerEvent) -> Option<LedgerState> {
    use LedgerEvent as E;
    use LedgerState as S;
    match event {
        E::Settle { .. } => match current {
            S::Reserved | S::Unknown => Some(S::Settled),
            S::Settled | S::Released => None,
        },
        E::Release => match current {
            S::Reserved | S::Unknown => Some(S::Released),
            S::Settled | S::Released => None,
        },
        E::MarkUnknown => match current {
            S::Reserved => Some(S::Unknown),
            S::Unknown => Some(S::Unknown),
            S::Settled | S::Released => None,
        },
    }
}

/// 该状态是否**仍占用预算**（`reserved` 与 `unknown` 都保留预留）。
pub const fn ledger_state_holds_budget(state: LedgerState) -> bool {
    matches!(state, LedgerState::Reserved | LedgerState::Unknown)
}

/// 该金额单位对应的最小单位名称（线上取值为 `creditMinor` / `usdMicros`）。
pub const fn currency_wire_name(currency: Currency) -> &'static str {
    match currency {
        Currency::CreditMinor => "creditMinor",
        Currency::UsdMicros => "usdMicros",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_parsing_is_exact_for_representable_literals() {
        assert_eq!(parse_decimal_scaled("30", 2, Rounding::Ceil).unwrap(), 3000);
        assert_eq!(
            parse_decimal_scaled("30.00", 2, Rounding::Ceil).unwrap(),
            3000
        );
        assert_eq!(parse_decimal_scaled("0.3", 2, Rounding::Ceil).unwrap(), 30);
        assert_eq!(parse_decimal_scaled("1.5", 2, Rounding::Ceil).unwrap(), 150);
        assert_eq!(parse_decimal_scaled(".5", 2, Rounding::Ceil).unwrap(), 50);
        assert_eq!(
            parse_decimal_scaled("30.", 2, Rounding::Ceil).unwrap(),
            3000
        );
        assert_eq!(
            parse_decimal_scaled(" 0.000001 ", 6, Rounding::Ceil).unwrap(),
            1
        );
        // 所有舍入模式在"可精确表示"时结果一致。
        for rounding in [Rounding::Floor, Rounding::HalfUp, Rounding::Ceil] {
            assert_eq!(parse_decimal_scaled("2.71", 2, rounding).unwrap(), 271);
        }
    }

    /// 舍入边界（QA 复算用）：余数恰半、略小于/大于半步、跨位进位。
    #[test]
    fn decimal_rounding_boundaries() {
        // 0.005 credit：小数位多于 scale（2）。
        assert_eq!(parse_decimal_scaled("0.005", 2, Rounding::Ceil).unwrap(), 1);
        assert_eq!(
            parse_decimal_scaled("0.005", 2, Rounding::HalfUp).unwrap(),
            1
        );
        assert_eq!(
            parse_decimal_scaled("0.005", 2, Rounding::Floor).unwrap(),
            0
        );
        // 0.0049 credit：Ceil 进 1、HalfUp/Floor 为 0。
        assert_eq!(
            parse_decimal_scaled("0.0049", 2, Rounding::Ceil).unwrap(),
            1
        );
        assert_eq!(
            parse_decimal_scaled("0.0049", 2, Rounding::HalfUp).unwrap(),
            0
        );
        // 17.9999999 USD：Ceil/HalfUp 进位到 18_000_000，Floor 截断。
        assert_eq!(
            parse_decimal_scaled("17.9999999", 6, Rounding::Ceil).unwrap(),
            18_000_000
        );
        assert_eq!(
            parse_decimal_scaled("17.9999999", 6, Rounding::HalfUp).unwrap(),
            18_000_000
        );
        assert_eq!(
            parse_decimal_scaled("17.9999999", 6, Rounding::Floor).unwrap(),
            17_999_999
        );
        // 0.0000005 USD = 0.5 micros：Ceil → 1；Floor → 0（不得静默逼近）。
        assert_eq!(
            parse_decimal_scaled("0.0000005", 6, Rounding::Ceil).unwrap(),
            1
        );
        assert_eq!(
            parse_decimal_scaled("0.0000005", 6, Rounding::Floor).unwrap(),
            0
        );
    }

    #[test]
    fn decimal_rejects_garbage_without_guessing() {
        for bad in [
            "", " ", "abc", "1e-6", "-1", "+1", "0.0.0", "1,000", "٣", "NaN",
        ] {
            assert!(
                parse_decimal_scaled(bad, 2, Rounding::Ceil).is_err(),
                "{bad:?} 应被拒绝"
            );
        }
        assert!(matches!(
            parse_decimal_scaled("-1", 2, Rounding::Ceil),
            Err(MoneyError::Negative { .. })
        ));
        assert!(matches!(
            parse_decimal_scaled("1e-6", 2, Rounding::Ceil),
            Err(MoneyError::InvalidDecimal { .. })
        ));
        assert!(parse_decimal_scaled(&"9".repeat(30), 6, Rounding::Ceil).is_err());
    }

    #[test]
    fn multiplication_rounds_up_never_down() {
        // 1 token × 0.25 USD / 1M tokens = 0.25 micros → 1 micro（保守）。
        assert_eq!(mul_div_ceil(1, 250_000, 1_000_000).unwrap(), 1);
        // 恰好整除时不多算。
        assert_eq!(mul_div_ceil(4, 250_000, 1_000_000).unwrap(), 1);
        assert_eq!(
            mul_div_ceil(4_000_000, 250_000, 1_000_000).unwrap(),
            1_000_000
        );
        assert!(mul_div_ceil(-1, 1, 1).is_err());
        assert!(mul_div_ceil(1, 1, 0).is_err());
    }

    #[test]
    fn formatting_matches_ui_contract() {
        assert_eq!(format_credits(3000), "30.00");
        assert_eq!(format_credits(1), "0.01");
        assert_eq!(format_credits(0), "0.00");
        assert_eq!(format_usd_micros(300_000), "0.30");
        assert_eq!(format_usd_micros(12_345), "0.012345");
        assert_eq!(format_usd_micros(120_000), "0.12");
        assert_eq!(format_usd_micros(1), "0.000001");
        assert_eq!(format_usd_micros(0), "0.00");
    }

    #[test]
    fn ledger_transitions_keep_unknown_reserved_and_forbid_resurrection() {
        use LedgerState::*;
        assert_eq!(
            next_ledger_state(Reserved, LedgerEvent::Settle { actual_minor: 30 }),
            Some(Settled)
        );
        assert_eq!(
            next_ledger_state(Reserved, LedgerEvent::Release),
            Some(Released)
        );
        assert_eq!(
            next_ledger_state(Reserved, LedgerEvent::MarkUnknown),
            Some(Unknown)
        );
        // unknown 不自动释放：只有显式事件才能改变；settle 也允许（对账后按实际结算）。
        assert_eq!(
            next_ledger_state(Unknown, LedgerEvent::MarkUnknown),
            Some(Unknown)
        );
        assert_eq!(
            next_ledger_state(Unknown, LedgerEvent::Settle { actual_minor: 7 }),
            Some(Settled)
        );
        assert_eq!(
            next_ledger_state(Unknown, LedgerEvent::Release),
            Some(Released)
        );
        // 终态：不允许复活或改写。
        assert_eq!(next_ledger_state(Settled, LedgerEvent::Release), None);
        assert_eq!(
            next_ledger_state(Settled, LedgerEvent::Settle { actual_minor: 1 }),
            None
        );
        assert_eq!(next_ledger_state(Released, LedgerEvent::MarkUnknown), None);
        // 预算占用：reserved 与 unknown 都占。
        assert!(ledger_state_holds_budget(Reserved));
        assert!(ledger_state_holds_budget(Unknown));
        assert!(!ledger_state_holds_budget(Settled));
        assert!(!ledger_state_holds_budget(Released));
        assert_eq!(currency_wire_name(Currency::CreditMinor), "creditMinor");
        assert_eq!(currency_wire_name(Currency::UsdMicros), "usdMicros");
    }
}
