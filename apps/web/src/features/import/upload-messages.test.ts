import { describe, expect, it } from "vitest";

import { describeUploadFailure } from "./upload-messages";
import { AssetUploadError } from "./upload";

describe("上传失败的可行动文案（UI-009/UI-010；A-14）", () => {
  it("磁盘预留不足：按 details.reason=insufficientStorage 显示所需/可用字节与清理提示", () => {
    const info = describeUploadFailure(
      new AssetUploadError({
        message: "磁盘预留空间不足：需要 10485760 字节，当前可用 1048576 字节",
        status: 413,
        code: "PAYLOAD_TOO_LARGE",
        details: { reason: "insufficientStorage", requiredBytes: 10_485_760, availableBytes: 1_048_576 },
      }),
    );
    expect(info.title).toBe("磁盘空间不足");
    expect(info.message).toContain("需要 10 MiB");
    expect(info.message).toContain("当前可用 1 MiB");
    expect(info.hint).toContain("清理磁盘");
  });

  it("物品累计上限与输入超限分开呈现", () => {
    const itemTotal = describeUploadFailure(
      new AssetUploadError({
        message: "物品累计体积将超过上限",
        status: 413,
        details: { reason: "itemTotalLimit", limitBytes: 1, currentBytes: 2, incomingBytes: 3 },
      }),
    );
    expect(itemTotal.title).toBe("物品累计体积超出上限");

    const tooLarge = describeUploadFailure(
      new AssetUploadError({ message: "照片超过 20 MiB 上限", status: 413 }),
    );
    expect(tooLarge.title).toBe("文件超过大小上限");
  });

  it("415 类型不支持给出转换提示；网络失败给出可重试提示", () => {
    const type = describeUploadFailure(
      new AssetUploadError({ message: "不支持的类型", status: 415 }),
    );
    expect(type.title).toBe("文件类型不支持");
    expect(type.hint).toContain("HEIC/WebP");

    const network = describeUploadFailure(
      new AssetUploadError({ message: "无法连接服务", status: null }),
    );
    expect(network.title).toBe("上传失败");
    expect(network.hint).toContain("后端进程");
  });

  it("取消不算失败卡片文案（由调用方决定是否展示）", () => {
    const info = describeUploadFailure(
      new AssetUploadError({ message: "上传已取消", status: null, aborted: true }),
    );
    expect(info.title).toBe("上传已取消");
    expect(info.hint).toBeNull();
  });
});
