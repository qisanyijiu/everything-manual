import { useEffect, useState } from "react";
import type { JobStageDto } from "../../api/endpoints";
import { useDocumentVisible } from "./jobs";

/** Local presentation only: reaching zero never requests a retry or a fresh quote. */
export function RetryCountdown({ safeRetry, nextRunAt }: Pick<JobStageDto, "safeRetry" | "nextRunAt">) {
  const visible = useDocumentVisible();
  const [now, setNow] = useState(Date.now);
  const deadline = nextRunAt == null ? Number.NaN : Date.parse(nextRunAt);
  useEffect(() => {
    setNow(Date.now());
    if (!visible || !Number.isFinite(deadline) || deadline <= Date.now()) return;
    const timer = window.setInterval(() => {
      const current = Date.now(); setNow(current);
      if (current >= deadline) window.clearInterval(timer);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [deadline, visible]);
  const seconds = Math.max(0, Math.ceil((deadline - now) / 1000));
  const time = !Number.isFinite(deadline) ? "等待调度，重试时间暂不可用" : seconds === 0 ? "等待调度更新" : `预计 ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")} 后重试`;
  return <div className="job-retry-wait" aria-live="off">
    <p>{safeRetry ? `第 ${safeRetry.number}/${safeRetry.limit} 次安全重试` : "安全重试次数暂不可用"}</p>
    <p data-testid="safe-retry-countdown">{time}</p>
  </div>;
}
