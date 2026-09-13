/**
 * T09 单元测试：PDF 拒绝路径的分类与文案（AC-024 / PRD §6.2 UI-016）。
 *
 * 加密与超页数的**可行动文案**在此逐字固定；真实 PDF.js 解析行为（`PasswordException`）
 * 与"不创建任何记录"的端到端证明在 Playwright e2e（`pdf-preparation.spec.ts`）。
 */

import { describe, expect, it } from "vitest";

import {
  classifyPdfError,
  ENCRYPTED_PDF_MESSAGE,
  INVALID_PDF_MESSAGE,
  tooManyPagesMessage,
  tooManyPagesRejection,
} from "./errors";

describe("classifyPdfError", () => {
  it("PasswordException → 加密 PDF 拒绝（含可行动文案）", () => {
    const error = new Error("need password");
    error.name = "PasswordException";
    const rejection = classifyPdfError(error);
    expect(rejection.kind).toBe("encrypted");
    expect(rejection.message).toBe(ENCRYPTED_PDF_MESSAGE);
    expect(rejection.message).toContain("解除加密");
  });

  it("其它错误 → 无法解析（不假装是加密问题）", () => {
    expect(classifyPdfError(new Error("Invalid PDF structure")).kind).toBe("invalid");
    expect(classifyPdfError(new Error("boom")).message).toBe(INVALID_PDF_MESSAGE);
  });
});

describe("页数上限", () => {
  it("文案包含实际页数与上限 100", () => {
    const message = tooManyPagesMessage(150);
    expect(message).toContain("150");
    expect(message).toContain("100");
  });

  it("拒绝结果携带实际页数（供 UI 显示）", () => {
    const rejection = tooManyPagesRejection(101);
    expect(rejection.kind).toBe("tooManyPages");
    expect(rejection.pageCount).toBe(101);
    expect(rejection.message).toBe(tooManyPagesMessage(101));
  });
});
