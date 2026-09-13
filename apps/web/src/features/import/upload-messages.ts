/**
 * 上传失败的可行动文案（PRD §6.2 UI-009/UI-010、§6.3.2）。
 *
 * 契约要点（T06 / A-14）：
 * - 413 必须按 `details.reason` 分流：`insufficientStorage`（磁盘预留不足，含
 *   `requiredBytes`/`availableBytes`，要显示所需/可用字节并给清理提示）与
 *   `itemTotalLimit`（物品累计上限）；两类都不能落成通用错误文案；
 * - 415/422/网络错误分别给"换文件 / 重试 / 检查连接"的下一步；
 * - 不做任何假成功：失败卡片保留，直到重试成功或用户移除。
 */

import { describeError } from "../../api/client";
import { formatBytes } from "../../lib/format";
import { isAssetUploadError } from "./upload";

export interface UploadFailureInfo {
  readonly title: string;
  /** 主文案（含服务端 message 或本模块的固定文案）。 */
  readonly message: string;
  /** 可行动提示（重试之外用户还能做什么）。 */
  readonly hint: string | null;
}

function readReason(details: unknown): string | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  const reason = (details as { reason?: unknown }).reason;
  return typeof reason === "string" ? reason : null;
}

function readNumber(details: unknown, key: string): number | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  const value = (details as Record<string, unknown>)[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** 把上传异常转成卡片上的标题/文案/下一步提示。 */
export function describeUploadFailure(error: unknown): UploadFailureInfo {
  if (isAssetUploadError(error)) {
    if (error.aborted) {
      return { title: "上传已取消", message: "该文件没有上传完成。", hint: null };
    }
    if (error.status === 415) {
      return {
        title: "文件类型不支持",
        message: error.message,
        hint: "说明书原件需要 PDF；照片需要 JPEG 或 PNG（HEIC/WebP 请先转换）。",
      };
    }
    if (error.status === 413) {
      const reason = readReason(error.details);
      if (reason === "insufficientStorage") {
        const required = readNumber(error.details, "requiredBytes");
        const available = readNumber(error.details, "availableBytes");
        const sizes =
          required !== null && available !== null
            ? `（需要 ${formatBytes(required)}，当前可用 ${formatBytes(available)}）`
            : "";
        return {
          title: "磁盘空间不足",
          message: `${error.message}${sizes}`,
          hint: "请清理磁盘空间或 data-dir 所在分区后重试；本次上传没有产生任何资产记录。",
        };
      }
      if (reason === "itemTotalLimit") {
        return {
          title: "物品累计体积超出上限",
          message: error.message,
          hint: "请先清理该物品的其他资料（本版本不提供删除入口，可先用更小的文件），再重试。",
        };
      }
      return {
        title: "文件超过大小上限",
        message: error.message,
        hint: "请压缩或换用更小的文件后重试。",
      };
    }
    if (error.status === 422) {
      return {
        title: "文件内容未通过校验",
        message: error.message,
        hint: "请确认文件完整、未被截断；损坏或超出像素上限的图片会被拒绝。",
      };
    }
    if (error.status === 404) {
      return {
        title: "上传目标不可用",
        message: error.message,
        hint: "请返回资料库确认物品仍存在，然后重新进入向导。",
      };
    }
  }

  const info = describeError(error);
  const network = info.status === null && info.code === null;
  return {
    title: "上传失败",
    message: info.message,
    hint: network ? "请确认后端进程正在运行后重试。" : "可重试；若持续失败请记录诊断请求 ID。",
  };
}
