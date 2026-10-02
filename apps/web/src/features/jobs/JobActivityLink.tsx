import { Link } from "react-router";
import { Icon } from "../../components/Icon";
import { useJobActivity } from "./jobs";

export function JobActivityLink() {
  const query = useJobActivity();
  const unavailable = query.isError;
  const count = query.data?.data.active;
  const label = unavailable ? "任务数暂不可用" : count === undefined ? "正在读取任务数" : `进行中任务 ${count} 个`;
  return <Link to="/jobs" className="job-activity-link" data-testid="job-activity-link" aria-label={label}>
    <Icon name="activity" size={18} />
    <span className="job-activity-link__label">{label}</span>
    <span className="job-activity-link__compact" aria-hidden="true">{unavailable ? "—" : count ?? "…"}</span>
  </Link>;
}
