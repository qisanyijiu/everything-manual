/**
 * 任务状态与阶段的可读映射（PRD §6.2 UI-029/UI-030/UI-038；REQ-031）。
 *
 * 硬约束（§6.3.2 禁用措辞清单）：
 * - **不出现"总进度 100%"**、不把阶段计数换算成线性百分比、不显示预计剩余时间；
 * - `succeeded` 一律写作"可复核草稿已产出（尚未发布）"——生成完成不等于已发布（ADR-005）；
 * - `submission_unknown` 写作"付费提交结果未知（等待对账）"，不得写成"失败"，
 *   也不得提供重试入口（对账入口由 `RecoveryPanel` 渲染）；
 * - 状态用**文本标签**表达，不只靠颜色或图标。
 *
 * 状态枚举来自 contracts.md §5（`queued/running/waiting_provider/retry_wait/needs_input/
 * submission_unknown/succeeded/failed/cancelled`）；未知取值原样显示（不猜测）。
 */

export type JobStatusCategory = "local" | "queue" | "provider" | "manual" | "unknown" | "done" | "failed";

export interface JobStatusMeta {
  /** 文本标签（界面显示；不出现虚假进度）。 */
  readonly label: string;
  /** 状态分类（UI-030：区分本地准备/排队/供应商进度/等待人工/失败/unknown）。 */
  readonly category: JobStatusCategory;
  /** 分类的可读名（列表筛选与分组用）。 */
  readonly categoryLabel: string;
  /** 该状态的下一步（"每种状态有可执行下一步"，REQ-031）。 */
  readonly nextStep: string;
}

const CATEGORY_LABELS: Record<JobStatusCategory, string> = {
  local: "本地准备/执行",
  queue: "排队",
  provider: "供应商进度",
  manual: "等待人工",
  unknown: "等待对账（结果未知）",
  done: "已结束",
  failed: "失败",
};

const STATUS_META: Record<string, JobStatusMeta> = {
  queued: {
    label: "排队中",
    category: "queue",
    categoryLabel: CATEGORY_LABELS.queue,
    nextStep: "已入队，等待执行器领取；需要停止时可取消任务。",
  },
  running: {
    label: "本地阶段进行中",
    category: "local",
    categoryLabel: CATEGORY_LABELS.local,
    nextStep: "应用正在执行本地阶段（准备/组装/校验）；可取消未提交的部分。",
  },
  waiting_provider: {
    label: "等待供应商",
    category: "provider",
    categoryLabel: CATEGORY_LABELS.provider,
    nextStep: "远端付费任务已提交，应用按间隔查询状态（可关闭浏览器，服务端继续）。",
  },
  retry_wait: {
    label: "退避重试中",
    category: "queue",
    categoryLabel: CATEGORY_LABELS.queue,
    nextStep:
      "上一次请求可安全重试，正在按退避序列等待；期间不提供「立即重试」按钮，以免打断退避。",
  },
  needs_input: {
    label: "等待人工补齐",
    category: "manual",
    categoryLabel: CATEGORY_LABELS.manual,
    nextStep: "已停止自动等待：请按缺项补齐资料，再按任务详情中的可用恢复动作继续（不会自动重复购买）。",
  },
  submission_unknown: {
    label: "付费提交结果未知（等待对账）",
    category: "unknown",
    categoryLabel: CATEGORY_LABELS.unknown,
    nextStep: "该分支后续购买已暂停：请先核对供应商账户并按对账动作处理（不提供盲目重试）。",
  },
  succeeded: {
    label: "已完成（可复核草稿已产出，尚未发布）",
    category: "done",
    categoryLabel: CATEGORY_LABELS.done,
    nextStep: "生成完成不等于已发布：需要人工确认知识并完成热点校准后才能发布。",
  },
  failed: {
    label: "失败",
    category: "failed",
    categoryLabel: CATEGORY_LABELS.failed,
    nextStep: "查看阶段错误摘要；可重试的阶段在任务详情给出入口（可用性以服务端判定为准）。",
  },
  cancelled: {
    label: "已取消",
    category: "done",
    categoryLabel: CATEGORY_LABELS.done,
    nextStep: "本地推进已停止；已提交给供应商的付费操作不会被撤销，账务与查询记录保留。",
  },
};

/** 未知/未来状态：原值照实显示，不猜测、不套用别的标签。 */
export function jobStatusMeta(status: string): JobStatusMeta {
  const meta = STATUS_META[status];
  if (meta !== undefined) {
    return meta;
  }
  return {
    label: `未知状态（${status}）`,
    category: "failed",
    categoryLabel: "未知状态",
    nextStep: "服务端返回了当前界面不认识的状态：请查看阶段明细与错误摘要，不猜测结果。",
  };
}

/** 终态（`succeeded/failed/cancelled`）：轮询必须停止（UI-029）。 */
export function isTerminalJobStatus(status: string): boolean {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

/** 阶段状态标签（UI-038：逐阶段展示，不用百分比）。 */
const STAGE_STATUS_LABELS: Record<string, string> = {
  queued: "排队中",
  running: "进行中",
  waiting_provider: "等待供应商",
  retry_wait: "退避重试中",
  needs_input: "缺项（等待人工）",
  submission_unknown: "结果未知（等待对账）",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

export function stageStatusLabel(status: string): string {
  return STAGE_STATUS_LABELS[status] ?? `未知状态（${status}）`;
}

/** 阶段种类的可读名（UI-038 的稳定核对点）。 */
export const STAGE_KIND_LABELS: Record<string, string> = {
  freeze_inputs: "冻结输入",
  manual_extract: "说明书提取（批次）",
  manual_merge: "知识合并",
  tripo_upload: "照片上传（Tripo）",
  tripo_submit: "模型生成提交（付费）",
  tripo_poll: "远端任务查询",
  model_download: "模型下载",
  model_validate: "模型校验",
  assemble_draft: "组装草稿",
};

export function stageKindLabel(kind: string, batchIndex: number): string {
  const base = STAGE_KIND_LABELS[kind] ?? kind;
  return kind === "manual_extract" && batchIndex > 0 ? `${base} #${batchIndex + 1}` : base;
}

/**
 * 重试被拒时的可行动文案（与服务端 `stages[].retry.message` 同源；
 * 这里只补一个"去哪儿做"的界面侧指引，不改变服务端判定）。
 */
export function retryDeniedHint(reason: string | null): string {
  switch (reason) {
    case "budgetNotHolding":
      return "该分支的预留已结算或释放：重试会重新请求供应商，需要重新获取报价并确认预算（或新建任务）。";
    case "branchSubmissionUnknown":
      return "同一分支存在未对账的付费提交：请先完成对账。";
    case "jobCancelled":
      return "任务已取消：取消后不再发起新的付费步骤。";
    case "stageNotRetryable":
      return "当前状态不是人工重试入口（只有失败或缺项的阶段可重试）。";
    default:
      return "该阶段当前不可重试：可用动作以服务端判定为准。";
  }
}
