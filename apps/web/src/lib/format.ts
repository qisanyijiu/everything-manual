/** 展示格式工具（PRD §6.3.3 U-02/U-04；API 一律 UTC RFC3339，展示用浏览器本地时区）。 */

const pad2 = (value: number): string => String(value).padStart(2, "0");

/**
 * 时间显示 `YYYY-MM-DD HH:mm`（浏览器本地时区）。
 * 服务端返回 UTC RFC3339（contracts.md §1）；无法解析时原样返回，不编造时间。
 */
export function formatLocalDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  const ymd = `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
  const hm = `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
  return `${ymd} ${hm}`;
}

const KIB = 1024;
const MIB = KIB * 1024;
const GIB = MIB * 1024;

/** 字节数显示（二进制单位；与 PRD §5.3 的 MiB 口径一致）。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return String(bytes);
  }
  if (bytes >= GIB) {
    return `${trimFixed(bytes / GIB)} GiB`;
  }
  if (bytes >= MIB) {
    return `${trimFixed(bytes / MIB)} MiB`;
  }
  if (bytes >= KIB) {
    return `${trimFixed(bytes / KIB)} KiB`;
  }
  return `${bytes} B`;
}

function trimFixed(value: number): string {
  const fixed = value.toFixed(1);
  return fixed.endsWith(".0") ? fixed.slice(0, -2) : fixed;
}
