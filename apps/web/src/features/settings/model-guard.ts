/** Keep this finite rule in sync with config/model_guard.rs (PC-06). */
export function isSuspectedCredentialModel(value: string): boolean {
  return /^sk-[A-Za-z0-9_-]{16,}$/.test(value.replace(/^[\p{White_Space}\uFEFF]+|[\p{White_Space}\uFEFF]+$/gu, "").replace(/^Bearer +/i, ""));
}

export const MODEL_FIELD_MESSAGE = "这里需要模型名称；API 密钥请在密钥操作中选择替换后输入";
export const HIDDEN_MODEL_MESSAGE = "模型需修正（疑似误填密钥，已隐藏）";
