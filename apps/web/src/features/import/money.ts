/**
 * 金额输入与展示的整数换算（PRD §6.3.3 U-04；contracts §1 金额单位）。
 *
 * 单位：Tripo `creditMinor` = 1/100 credit（scale 2）；说明书 AI `usdMicros` = 1/1,000,000 USD（scale 6）。
 * 全部用整数运算（十进制字符串 → 最小单位整数），**没有浮点累加**：
 * 用户输入与展示都经过本模块，服务端始终以整数最小单位为权威值。
 *
 * 超过 scale 位小数的输入**不静默取整**：返回字段级错误让用户改成可精确表示的值
 * （避免"显示的授权上限"与"实际授权值"不一致）。
 */

export const CREDIT_MINOR_SCALE = 2;
export const USD_MICROS_SCALE = 6;

export interface AmountParseResult {
  readonly ok: boolean;
  /** 解析成功时的整数最小单位。 */
  readonly minor: number;
  /** 解析失败时的字段级文案。 */
  readonly message: string | null;
}

const FAILED = (message: string): AmountParseResult => ({ ok: false, minor: 0, message });

/** 十进制字符串 → 整数最小单位（整数运算，无浮点）。 */
export function parseMinorInput(value: string, scale: number): AmountParseResult {
  const trimmed = value.trim();
  if (trimmed === "") {
    return FAILED("请输入金额");
  }
  const match = /^(\d+)(?:\.(\d*))?$/.exec(trimmed);
  if (match === null) {
    return FAILED("金额只能是数字（最多一个小数点，不支持负号、科学计数法或千分位）");
  }
  const integerPart = match[1] ?? "";
  const fractionPart = match[2] ?? "";
  if (fractionPart.length > scale) {
    return FAILED(`最多 ${scale} 位小数（该金额单位最小到 1/${10 ** scale}）`);
  }
  const padded = fractionPart.padEnd(scale, "0");
  const minor = Number(integerPart) * 10 ** scale + Number(padded === "" ? "0" : padded);
  if (!Number.isSafeInteger(minor)) {
    return FAILED("金额过大，请填写更小的值");
  }
  return { ok: true, minor, message: null };
}

/**
 * 整数最小单位 → 可编辑的十进制字符串（用于预算输入框的默认值）。
 *
 * credits 固定两位小数；USD 去掉末尾多余的 0（但至少保留两位小数），
 * 保证"复制输入框的值回填"能解析出同一个最小单位（不丢精度）。
 */
export function minorToInputString(minor: number, scale: number): string {
  const negative = minor < 0;
  const absolute = Math.abs(Math.trunc(minor));
  const factor = 10 ** scale;
  const whole = Math.floor(absolute / factor);
  const fraction = String(absolute % factor).padStart(scale, "0");
  let renderedFraction = fraction.replace(/0+$/u, "");
  while (renderedFraction.length < 2) {
    renderedFraction += "0";
  }
  return `${negative ? "-" : ""}${whole}.${renderedFraction}`;
}
