/**
 * 通用 412 并发冲突恢复（PRD §6.2 UI-008 / UI-056）：
 * 显示 `details.currentRevision`、提供「刷新后重试」、不自动覆盖服务端内容；
 * 表单输入由调用方保留（本组件不触碰表单状态）；刷新前调用方须禁用原提交按钮。
 */

export function ConflictNotice({
  currentRevision,
  onRefresh,
  refreshing = false,
  description,
}: {
  currentRevision: number | null;
  onRefresh: () => void;
  refreshing?: boolean;
  description?: string;
}) {
  const headline =
    currentRevision === null
      ? "该内容已被其他操作更新"
      : `该内容已被其他操作更新（当前 r${currentRevision}）`;
  return (
    <div className="conflict-notice" role="alert">
      <p className="conflict-notice__headline">{headline}</p>
      <p className="conflict-notice__detail">
        {description ?? "你已填写的内容会保留，本页不会自动覆盖服务端的最新版本。"}
      </p>
      <button type="button" onClick={onRefresh} disabled={refreshing}>
        {refreshing ? "正在刷新…" : "刷新后重试"}
      </button>
    </div>
  );
}
