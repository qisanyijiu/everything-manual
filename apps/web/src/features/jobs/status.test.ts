/**
 * T17 状态与文案映射测试（PRD §6.2 UI-029/UI-030/UI-038；§6.3.2 禁用措辞清单）。
 *
 * 覆盖：
 * - 九个运行时状态各有独立文本标签与"下一步"，未知状态原值照实显示；
 * - `succeeded` 写作"可复核草稿已产出，尚未发布"；
 * - `submission_unknown` 不写成失败，且提示先对账、不盲目重试；
 * - 终态判定（轮询停止的判据）；
 * - 禁用措辞清单不出现在这些文案里（"总进度 100%"、"自动发布"、"离线可用"……）。
 */

import { describe, expect, it } from "vitest";

import {
  isTerminalJobStatus,
  jobStatusMeta,
  retryDeniedHint,
  stageKindLabel,
  stageStatusLabel,
} from "./status";

const ALL_STATUSES = [
  "queued",
  "running",
  "waiting_provider",
  "retry_wait",
  "needs_input",
  "submission_unknown",
  "succeeded",
  "failed",
  "cancelled",
];

const FORBIDDEN_WORDING = [
  "已自动校准",
  "自动发布",
  "总进度 100%",
  "已证明页图来自原 PDF",
  "供应商账户硬封顶",
  "零费用",
  "重试不会重复收费",
  "离线可用",
];

describe("任务状态映射（UI-030）", () => {
  it("九个状态各有文本标签与下一步，分类互相区分", () => {
    const labels = new Set<string>();
    for (const status of ALL_STATUSES) {
      const meta = jobStatusMeta(status);
      expect(meta.label.length).toBeGreaterThan(0);
      expect(meta.nextStep.length).toBeGreaterThan(0);
      labels.add(meta.label);
    }
    expect(labels.size).toBe(ALL_STATUSES.length);

    expect(jobStatusMeta("submission_unknown").category).toBe("unknown");
    expect(jobStatusMeta("needs_input").category).toBe("manual");
    expect(jobStatusMeta("waiting_provider").category).toBe("provider");
    expect(jobStatusMeta("failed").category).toBe("failed");
  });

  it("succeeded 明确写出「可复核草稿已产出，尚未发布」，不说已发布", () => {
    const meta = jobStatusMeta("succeeded");
    expect(meta.label).toContain("可复核草稿已产出");
    expect(meta.label).toContain("尚未发布");
    expect(meta.nextStep).toContain("不等于已发布");
  });

  it("submission_unknown 提示先对账，不写成失败、不承诺可重试", () => {
    const meta = jobStatusMeta("submission_unknown");
    expect(meta.label).toContain("结果未知");
    expect(meta.label).not.toContain("失败");
    expect(meta.nextStep).toContain("先核对");
    expect(meta.nextStep).toContain("不提供盲目重试");
  });

  it("未知状态原值照实显示（不猜测、不套用别的标签）", () => {
    const meta = jobStatusMeta("some_future_status");
    expect(meta.label).toContain("some_future_status");
    expect(meta.nextStep).toContain("不猜测");
  });

  it("终态只有 succeeded/failed/cancelled（轮询停止判据）", () => {
    expect(isTerminalJobStatus("succeeded")).toBe(true);
    expect(isTerminalJobStatus("failed")).toBe(true);
    expect(isTerminalJobStatus("cancelled")).toBe(true);
    for (const status of ["queued", "running", "waiting_provider", "retry_wait", "needs_input", "submission_unknown"]) {
      expect(isTerminalJobStatus(status)).toBe(false);
    }
  });
});

describe("阶段标签与重试被拒说明", () => {
  it("阶段状态与种类有可读名，批次带序号", () => {
    expect(stageStatusLabel("needs_input")).toContain("缺项");
    expect(stageStatusLabel("submission_unknown")).toContain("对账");
    expect(stageKindLabel("manual_extract", 0)).toBe("说明书提取（批次）");
    expect(stageKindLabel("manual_extract", 2)).toBe("说明书提取（批次） #3");
    expect(stageKindLabel("model_validate", 0)).toBe("模型校验");
    expect(stageStatusLabel("future_state")).toContain("future_state");
  });

  it("budgetNotHolding 的说明指向重新报价，不写「可重试」", () => {
    const hint = retryDeniedHint("budgetNotHolding");
    expect(hint).toContain("重新获取报价");
    expect(hint).not.toContain("可重试");
  });

  it("禁用措辞清单不出现在状态/阶段文案里", () => {
    const text = [
      ...ALL_STATUSES.flatMap((status) => [
        jobStatusMeta(status).label,
        jobStatusMeta(status).nextStep,
        jobStatusMeta(status).categoryLabel,
      ]),
      ...[
        "queued",
        "running",
        "waiting_provider",
        "retry_wait",
        "needs_input",
        "submission_unknown",
        "succeeded",
        "failed",
        "cancelled",
      ].map(stageStatusLabel),
      ...["budgetNotHolding", "branchSubmissionUnknown", "jobCancelled", "stageNotRetryable", null].map(
        retryDeniedHint,
      ),
    ].join("\n");
    for (const forbidden of FORBIDDEN_WORDING) {
      expect(text).not.toContain(forbidden);
    }
  });
});
