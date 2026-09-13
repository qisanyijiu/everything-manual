import { describe, expect, it } from "vitest";

import {
  CREDIT_MINOR_SCALE,
  USD_MICROS_SCALE,
  minorToInputString,
  parseMinorInput,
} from "./money";

describe("金额输入解析（整数最小单位，无浮点）", () => {
  it("credits（scale=2）：整数与两位小数都精确换算", () => {
    expect(parseMinorInput("30", CREDIT_MINOR_SCALE)).toEqual({ ok: true, minor: 3000, message: null });
    expect(parseMinorInput("30.00", CREDIT_MINOR_SCALE)).toEqual({
      ok: true,
      minor: 3000,
      message: null,
    });
    expect(parseMinorInput("0.01", CREDIT_MINOR_SCALE)).toEqual({ ok: true, minor: 1, message: null });
    expect(parseMinorInput(" 45.5 ", CREDIT_MINOR_SCALE)).toEqual({
      ok: true,
      minor: 4550,
      message: null,
    });
  });

  it("USD（scale=6）：微美元精确换算，不引入浮点误差", () => {
    expect(parseMinorInput("0.019992", USD_MICROS_SCALE)).toEqual({
      ok: true,
      minor: 19992,
      message: null,
    });
    expect(parseMinorInput("1", USD_MICROS_SCALE)).toEqual({
      ok: true,
      minor: 1_000_000,
      message: null,
    });
  });

  it("超过 scale 位小数不静默取整，返回字段级错误", () => {
    const result = parseMinorInput("0.005", CREDIT_MINOR_SCALE);
    expect(result.ok).toBe(false);
    expect(result.message).toContain("最多 2 位小数");
  });

  it("拒绝非法输入（空、负数、科学计数法、千分位、多个小数点）", () => {
    for (const value of ["", "  ", "-1", "1e3", "1,000", "1..2", "abc", "1.2.3", "$5"]) {
      expect(parseMinorInput(value, CREDIT_MINOR_SCALE).ok, `输入 ${JSON.stringify(value)} 应被拒绝`).toBe(
        false,
      );
    }
  });

  it("回填字符串能被解析回同一最小单位（展示不丢精度）", () => {
    const samples: readonly (readonly [number, number])[] = [
      [3000, CREDIT_MINOR_SCALE],
      [19992, USD_MICROS_SCALE],
      [1, USD_MICROS_SCALE],
      [1_000_000, USD_MICROS_SCALE],
      [0, CREDIT_MINOR_SCALE],
      [4550, CREDIT_MINOR_SCALE],
    ];
    for (const [minor, scale] of samples) {
      const rendered = minorToInputString(minor, scale);
      const parsed = parseMinorInput(rendered, scale);
      expect(parsed.ok, `${rendered} 应可解析`).toBe(true);
      expect(parsed.minor).toBe(minor);
    }
  });

  it("credits 展示固定两位小数，USD 至少两位", () => {
    expect(minorToInputString(3000, CREDIT_MINOR_SCALE)).toBe("30.00");
    expect(minorToInputString(1, CREDIT_MINOR_SCALE)).toBe("0.01");
    expect(minorToInputString(19992, USD_MICROS_SCALE)).toBe("0.019992");
    expect(minorToInputString(1_000_000, USD_MICROS_SCALE)).toBe("1.00");
  });
});
