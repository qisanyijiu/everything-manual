import { ApiError, requestBytes } from "./client";
import { API_PREFIX } from "./endpoints";

export const INVALID_RELEASE_ARCHIVE = "INVALID_RELEASE_ARCHIVE";

/** Never turn response headers into a path, or expose an unsafe candidate in the UI. */
export function releaseDownloadFilename(disposition: string | null, releaseId: string): string {
  const safeId = releaseId.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 128) || "download";
  const fallback = `release-${safeId}.zip`;
  if (disposition === null) return fallback;
  const extended = /(?:^|;)\s*filename\*\s*=\s*([^;]*)/i.exec(disposition);
  const ordinary = /(?:^|;)\s*filename\s*=\s*("[^"]*"|[^;]*)/i.exec(disposition);
  let candidate: string;
  if (extended !== null) {
    const value = /^UTF-8'[^']*'(.+)$/i.exec(extended[1]?.trim() ?? "");
    if (value === null) return fallback;
    try { candidate = decodeURIComponent(value[1] ?? ""); } catch { return fallback; }
  } else if (ordinary !== null) {
    const value = ordinary[1]?.trim() ?? "";
    candidate = value.startsWith('"') ? value.slice(1, -1) : value;
  } else {
    return fallback;
  }
  // A conservative portable basename: no separators, control/bidi characters,
  // drive notation, hidden names, reserved device names or ambiguous suffixes.
  return candidate.length <= 180 &&
    /^[\p{L}\p{N}][\p{L}\p{N}\p{M} _().-]*\.zip$/iu.test(candidate) &&
    !/^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(candidate)
    ? candidate
    : fallback;
}

/**
 * Validate the ZIP envelope before offering bytes to the browser. The server
 * owns entry/manifest/hash validation; here MIME + local/central/end records
 * prevent successful HTML/JSON or a truncated response becoming a fake ZIP.
 */
function hasZipEnvelope(bytes: ArrayBuffer): boolean {
  const view = new DataView(bytes);
  if (view.byteLength < 22 + 30 + 46 || view.getUint32(0, true) !== 0x04034b50) return false;
  const earliest = Math.max(0, view.byteLength - 22 - 65535);
  for (let offset = view.byteLength - 22; offset >= earliest; offset -= 1) {
    if (view.getUint32(offset, true) !== 0x06054b50) continue;
    const entries = view.getUint16(offset + 10, true);
    const directoryLength = view.getUint32(offset + 12, true);
    const directoryOffset = view.getUint32(offset + 16, true);
    return offset + 22 + view.getUint16(offset + 20, true) === view.byteLength &&
      view.getUint16(offset + 4, true) === 0 && view.getUint16(offset + 6, true) === 0 &&
      entries > 0 && entries === view.getUint16(offset + 8, true) &&
      directoryLength >= 46 && directoryOffset >= 30 &&
      directoryOffset + directoryLength === offset &&
      view.getUint32(directoryOffset, true) === 0x02014b50;
  }
  return false;
}

export async function fetchReleaseArchive(releaseId: string, signal: AbortSignal) {
  const response = await requestBytes(
    `${API_PREFIX}/releases/${encodeURIComponent(releaseId)}/export`,
    { signal, cache: "no-store" },
  );
  const mime = response.contentType?.split(";", 1)[0]?.trim().toLowerCase();
  if ((mime !== "application/zip" && mime !== "application/x-zip-compressed") ||
    !hasZipEnvelope(response.bytes)) {
    throw new ApiError(200, INVALID_RELEASE_ARCHIVE,
      "服务返回的资料包格式不正确，未开始下载。请重试。", response.headers.get("x-request-id"));
  }
  return {
    blob: new Blob([response.bytes], { type: "application/zip" }),
    filename: releaseDownloadFilename(response.headers.get("content-disposition"), releaseId),
  };
}

/** Keep unknown server messages and error pages out of download feedback. */
export function releaseDownloadError(error: unknown): { message: string; requestId: string | null } {
  const requestId = error instanceof ApiError && error.requestId !== null &&
    /^[a-zA-Z0-9_-]{1,128}$/.test(error.requestId) ? error.requestId : null;
  if (!(error instanceof ApiError)) {
    return { message: "无法连接服务，资料包未下载。请检查连接后重试。", requestId };
  }
  const message = error.code === INVALID_RELEASE_ARCHIVE
    ? "服务返回的资料包格式不正确，未开始下载。请重试。"
    : error.status === 403 ? "没有下载该发布版本的权限。"
      : error.status === 404 ? "该发布版本已无法找到，请返回版本列表确认。"
        : error.status === 401 ? "登录已失效，请重新登录后下载。"
          : `资料包下载失败（HTTP ${error.status}），请稍后重试。`;
  return { message, requestId };
}
