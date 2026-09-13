/**
 * 准备记录的会话指针（ADR-003 / T09 §T09-6）。
 *
 * `sessionStorage` 里只保存 **preparation id 指针**（"该查哪条记录"），
 * 页状态的权威来源始终是 `GET /preparations/{id}`（contracts §3：不假定浏览器存储是事实来源）。
 *
 * 为什么需要它：服务端没有"按 document 列出 preparation"的读取端点，
 * 向导第 5 步（报价/确认）需要一个 **ready** 的 preparationId；同一个标签页会话语境下
 * 由第 4 步写入这里、第 5 步读取。指针失效（换浏览器/清存储）时界面按"准备未完成"处理，
 * 不伪造准备状态。该限制记录在 implementation.md §T16。
 */

export function preparationPointerKey(itemId: string): string {
  return `em.prepare.${itemId}`;
}

export function rememberPreparationId(itemId: string, preparationId: string): void {
  try {
    window.sessionStorage.setItem(preparationPointerKey(itemId), preparationId);
  } catch {
    // 隐私模式/存储被禁用：忽略，准备流程本身不依赖它。
  }
}

export function recallPreparationId(itemId: string): string | null {
  try {
    return window.sessionStorage.getItem(preparationPointerKey(itemId));
  } catch {
    return null;
  }
}

export function forgetPreparationId(itemId: string): void {
  try {
    window.sessionStorage.removeItem(preparationPointerKey(itemId));
  } catch {
    // 同上：忽略。
  }
}
