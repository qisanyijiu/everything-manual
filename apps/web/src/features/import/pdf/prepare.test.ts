/**
 * T09 单元测试：PDF 准备的纯逻辑（不依赖浏览器 canvas / PDF.js worker）。
 *
 * 覆盖（PRD 修订 2 / ui_revision 2）：
 * - 页文字拼接（扫描页判定依赖"空文本"语义）；
 * - 渲染缩放（长边 ≤2000px、不过度放大）；
 * - 缺页计算（1-based、只补缺失页 = AC-023 的客户端一半）；
 * - 页资产文件名的稳定格式。
 * 真实 PDF.js 渲染、CMaps/WASM 本地加载与逐页上传在 Playwright e2e 覆盖
 * （见 `tests/e2e/pdf-preparation.spec.ts`）。
 */

import { describe, expect, it } from "vitest";

import { missingPageNumbers, pageAssetFilename, renderScaleFor, textItemsToText } from "./prepare";
import { MAX_PAGE_IMAGE_LONG_EDGE } from "./vendor";

describe("textItemsToText", () => {
  it("按阅读顺序拼接并在 hasEOL 处换行", () => {
    const text = textItemsToText([
      { str: "Step 1: Loosen", hasEOL: false },
      { str: "the screws.", hasEOL: true },
      { str: "Step 2: Remove cover." },
    ]);
    expect(text).toBe("Step 1: Loosen the screws.\nStep 2: Remove cover.");
  });

  it("空项（扫描页没有文字层）得到空字符串", () => {
    expect(textItemsToText([])).toBe("");
    expect(textItemsToText([{ str: "   " }, { str: "" }])).toBe("");
  });

  it("保留非拉丁字符（不做任何转写）", () => {
    expect(textItemsToText([{ str: "部件一：松开螺丝", hasEOL: true }])).toBe(
      "部件一：松开螺丝",
    );
  });
});

describe("renderScaleFor", () => {
  it("长边超过 2000px 时缩到上限", () => {
    const scale = renderScaleFor(4000, 3000);
    expect(4000 * scale).toBeLessThanOrEqual(MAX_PAGE_IMAGE_LONG_EDGE);
    expect(4000 * scale).toBeCloseTo(MAX_PAGE_IMAGE_LONG_EDGE, 5);
  });

  it("小页面最多放大 2 倍（不产生无意义的大图）", () => {
    expect(renderScaleFor(200, 100)).toBe(2);
    expect(renderScaleFor(595, 842)).toBeCloseTo(2, 5);
  });

  it("非常规尺寸不会返回 0 或负数", () => {
    expect(renderScaleFor(0, 0)).toBe(1);
    expect(renderScaleFor(2001, 100)).toBeLessThanOrEqual(MAX_PAGE_IMAGE_LONG_EDGE / 2001 + 1e-9);
  });
});

describe("missingPageNumbers", () => {
  it("只返回缺失页（1-based，升序）", () => {
    expect(missingPageNumbers(5, [1, 2, 3, 4, 5])).toEqual([]);
    expect(missingPageNumbers(5, [1, 2])).toEqual([3, 4, 5]);
    expect(missingPageNumbers(5, [2, 4])).toEqual([1, 3, 5]);
    expect(missingPageNumbers(3, [])).toEqual([1, 2, 3]);
  });

  it("忽略服务端返回的越界页号（不制造负页号）", () => {
    expect(missingPageNumbers(2, [1, 2, 7])).toEqual([]);
  });
});

describe("pageAssetFilename", () => {
  it("页号补零到 4 位，扩展名可控", () => {
    expect(pageAssetFilename(1, "jpg")).toBe("page-0001.jpg");
    expect(pageAssetFilename(100, "txt")).toBe("page-0100.txt");
  });
});
