/**
 * PDF 打开失败的分类与可行动文案（T09 / AC-024；PRD §6.2 UI-016）。
 *
 * 分工（ADR-003）：加密与"实际页数超限"只有浏览器里的 PDF.js 能权威判定，
 * 因此**拒绝发生在客户端**：不创建页记录、不进入 jobs、不产生任何收费请求
 * （`PreparePage` 在拿到本模块的拒绝结果时**不会**调用 `POST .../preparations`）。
 *
 * 文案与环境无关的常量集中在此，便于 QA 与单测逐字核对（禁用措辞清单见 PRD §6.3.2：
 * 不写"已自动处理"之类的假承诺；只说明原因与下一步）。
 */

import { MAX_PDF_PAGES } from "./vendor";

/** 拒绝原因（结构化，供 UI 分支与测试断言）。 */
export type PdfRejectionKind = "encrypted" | "tooManyPages" | "invalid";

export interface PdfRejection {
  readonly kind: PdfRejectionKind;
  readonly message: string;
  /** 超页数时携带实际页数（文案里含 N）。 */
  readonly pageCount?: number;
}

/** 加密 PDF 的固定文案（UI-016 逐字要求）。 */
export const ENCRYPTED_PDF_MESSAGE =
  "该 PDF 已加密，首版不支持，请先解除加密后再上传";

/** 超过 100 页的文案（含实际页数）。 */
export function tooManyPagesMessage(pageCount: number): string {
  return `PDF 共 ${pageCount} 页，超过 ${MAX_PDF_PAGES} 页上限`;
}

/** 其它无法解析的 PDF（损坏、非 PDF 内容）。 */
export const INVALID_PDF_MESSAGE =
  "该文件无法作为 PDF 解析：请确认上传的是完整、未损坏的 PDF 原件";

/**
 * 把 PDF.js 打开/解析阶段的异常分类成可行动结果。
 *
 * PDF.js 的加密与密码错误使用 `PasswordException`：
 * - `NEED_PASSWORD`（需要密码，未提供）→ 加密 PDF 拒绝；
 * - `INCORRECT_PASSWORD`（提供了密码但不对）→ 同样按"已加密"拒绝
 *   （首版不提供密码输入，不存在"密码错误"这一用户动作）。
 */
export function classifyPdfError(error: unknown): PdfRejection {
  const name = (error as { name?: unknown } | null)?.name;
  if (name === "PasswordException") {
    return { kind: "encrypted", message: ENCRYPTED_PDF_MESSAGE };
  }
  return { kind: "invalid", message: INVALID_PDF_MESSAGE };
}

/** 页数超限的拒绝结果。 */
export function tooManyPagesRejection(pageCount: number): PdfRejection {
  return { kind: "tooManyPages", message: tooManyPagesMessage(pageCount), pageCount };
}

/** 用户取消（关闭/切换/取消准备）不是错误，用于把取消与失败分开。 */
export class PrepareCancelledError extends Error {
  constructor() {
    super("准备已取消");
    this.name = "PrepareCancelledError";
  }
}

export function isCancelled(error: unknown): boolean {
  return error instanceof PrepareCancelledError;
}
