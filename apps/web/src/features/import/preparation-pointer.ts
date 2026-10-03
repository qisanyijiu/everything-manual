/** Non-secret lookup hint only. Discovery and GET verify all progress, identity and compatibility. */

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
