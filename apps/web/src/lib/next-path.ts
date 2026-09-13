/**
 * `next` 返回路径的安全校验（PRD §6.1.1）：
 * 只接受同源相对路径；非相对、协议相对（`//host`）、绝对 URL、反斜杠变体一律回落 `/`。
 * 该函数是纯函数，任何来源（URL 查询参数、router state）都要经过它。
 */

/** 控制字符会截断/混淆浏览器解析（如 `/foo\nbar`），一律拒绝。 */
function hasControlCharacters(value: string): boolean {
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 0x20 || code === 0x7f) {
      return true;
    }
  }
  return false;
}

export function safeNextPath(raw: string | null | undefined, fallback = "/"): string {
  if (typeof raw !== "string" || raw === "") {
    return fallback;
  }
  if (hasControlCharacters(raw)) {
    return fallback;
  }
  if (!raw.startsWith("/")) {
    return fallback;
  }
  // 协议相对 `//evil.example` 与反斜杠变体 `/\evil.example` 都会跳出同源。
  if (raw.startsWith("//") || raw.startsWith("/\\")) {
    return fallback;
  }
  if (raw.includes("://") || raw.includes("\\")) {
    return fallback;
  }
  return raw;
}

/** 由当前 location 生成站内相对路径（`pathname + search`），用于 401 后保留返回位置。 */
export function currentRelativePath(location: { pathname: string; search: string }): string {
  return `${location.pathname}${location.search}`;
}

/** 生成登录页地址：`/login?next=<站内相对路径>`（`next` 为 `/` 时不带参数）。 */
export function loginHref(location: { pathname: string; search: string }): string {
  const next = safeNextPath(currentRelativePath(location));
  if (next === "/") {
    return "/login";
  }
  return `/login?next=${encodeURIComponent(next)}`;
}
