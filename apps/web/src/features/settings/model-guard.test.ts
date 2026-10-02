import { describe, expect, it } from "vitest";
import { isSuspectedCredentialModel } from "./model-guard";
describe("PC06 与后端相同的有限模型规则", () => {
  it("匹配边界和Bearer形式，不扫描任意子串", () => {
    for (const value of ["sk-0123456789abcdef", " sk-0123456789abcdef ", "bEaReR  sk-proj-fake_canary_12345", "\uFEFFsk-0123456789abcdef\uFEFF", "\u0085sk-0123456789abcdef\u0085"]) expect(isSuspectedCredentialModel(value)).toBe(true);
    for (const value of ["sk-0123456789abcde", "sk-local", "org/custom-model", "local:model-v2", "SK-0123456789abcdef", "Bearer\tsk-0123456789abcdef", "text sk-0123456789abcdef", "sk-0123456789abcdef.", "sk-0123456789abcde中文"]) expect(isSuspectedCredentialModel(value)).toBe(false);
  });
});
