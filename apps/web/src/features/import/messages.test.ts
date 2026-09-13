/**
 * T09 单元测试：封存失败明细的可读化（UI-018：422 逐条列出缺项）。
 */

import { describe, expect, it } from "vitest";

import { describeCompleteFailure } from "./messages";

describe("describeCompleteFailure", () => {
  it("缺页 → 列出页号", () => {
    expect(describeCompleteFailure({ reason: "incompletePages", missingPages: [2, 5] })).toEqual([
      "缺页：第 2、5 页",
    ]);
  });

  it("资产问题 → 逐页列出原因", () => {
    expect(
      describeCompleteFailure({
        reason: "assetMismatch",
        pages: [
          { pageNumber: 1, problem: "页图资产不可用：内容不可用（storage_state=quarantined）" },
          { pageNumber: 3, problem: "缺少页图资产（扫描页也需要上传页图）" },
        ],
      }),
    ).toEqual([
      "第 1 页：页图资产不可用：内容不可用（storage_state=quarantined）",
      "第 3 页：缺少页图资产（扫描页也需要上传页图）",
    ]);
  });

  it("无明细/异形载荷 → 空数组（不编造缺项）", () => {
    expect(describeCompleteFailure(null)).toEqual([]);
    expect(describeCompleteFailure({})).toEqual([]);
    expect(describeCompleteFailure({ missingPages: [] })).toEqual([]);
    expect(describeCompleteFailure({ pages: [{ pageNumber: "1" }] })).toEqual([]);
  });
});
