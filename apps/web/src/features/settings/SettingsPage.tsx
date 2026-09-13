/**
 * 设置与状态（PRD §6.2 UI-003；`GET /settings/status` + `/health/live` + `/health/ready`）。
 *
 * - 只读状态：providersConfigured、limits、capabilities、存活与就绪自检；
 * - 未配置项标注「未配置」并说明「生成与报价不可用，已有资料仍可读」；
 * - **不提供**网页填写密钥的输入框，也不提供 TLS/证书/监听配置入口
 *   （部署边界＝回环或受信反向代理，D-4/A-15）；页面不含任何密钥字符；
 * - 失败：「无法读取服务状态」+ requestId + 重试；状态不只靠颜色表达。
 */

import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { fetchHealthLive, fetchReadiness, fetchSettingsStatus } from "../../api/endpoints";
import { Skeleton } from "../../components/Skeleton";
import { formatBytes } from "../../lib/format";

export function SettingsPage() {
  const statusQuery = useQuery({
    queryKey: ["settings", "status"],
    queryFn: fetchSettingsStatus,
  });
  const liveQuery = useQuery({
    queryKey: ["health", "live"],
    queryFn: fetchHealthLive,
  });
  const readyQuery = useQuery({
    queryKey: ["health", "ready"],
    queryFn: fetchReadiness,
  });

  return (
    <section className="page settings-page" aria-labelledby="settings-title">
      <h1 id="settings-title">设置与状态</h1>
      <p className="page__lead">
        本页只展示服务端配置状态，不显示也不接受任何密钥；密钥由服务端配置文件或环境注入。
      </p>

      <section className="panel" aria-labelledby="providers-title">
        <h2 id="providers-title">供应商配置</h2>
        {statusQuery.isPending && <Skeleton label="正在读取服务状态…" rows={2} />}
        {statusQuery.isError && (
          <div className="error-panel" role="alert">
            <p>无法读取服务状态。{describeError(statusQuery.error).message}</p>
            {describeError(statusQuery.error).requestId !== null && (
              <p>
                诊断请求 ID：<code>{describeError(statusQuery.error).requestId}</code>
              </p>
            )}
            <button type="button" onClick={() => void statusQuery.refetch()}>
              重试
            </button>
          </div>
        )}
        {statusQuery.data !== undefined && (
          <ul className="status-list">
            <li>
              <span className="status-list__name">Tripo（模型生成）</span>
              <span className="status-label">
                {statusQuery.data.data.providersConfigured.tripo ? "已配置" : "未配置"}
              </span>
            </li>
            <li>
              <span className="status-list__name">说明书 AI</span>
              <span className="status-label">
                {statusQuery.data.data.providersConfigured.manualAi ? "已配置" : "未配置"}
              </span>
            </li>
            <li>
              <span className="status-list__name">生成能力</span>
              <span className="status-label">
                {statusQuery.data.data.capabilities.generation ? "可用" : "不可用"}
              </span>
            </li>
          </ul>
        )}
        {statusQuery.data !== undefined &&
          (!statusQuery.data.data.providersConfigured.tripo ||
            !statusQuery.data.data.providersConfigured.manualAi) && (
            <p className="status-note" role="status">
              未配置的供应商：生成与报价不可用；已有资料仍可读。
            </p>
          )}
      </section>

      <section className="panel" aria-labelledby="limits-title">
        <h2 id="limits-title">生效的输入限制</h2>
        {statusQuery.isPending && <Skeleton label="正在读取限制…" rows={2} />}
        {statusQuery.data !== undefined && (
          <dl className="meta-list">
            <div>
              <dt>原 PDF 大小</dt>
              <dd>{formatBytes(statusQuery.data.data.limits.maxPdfBytes)}</dd>
            </div>
            <div>
              <dt>原 PDF 页数</dt>
              <dd>{statusQuery.data.data.limits.maxPdfPages} 页</dd>
            </div>
            <div>
              <dt>照片单文件</dt>
              <dd>{formatBytes(statusQuery.data.data.limits.maxPhotoBytes)}</dd>
            </div>
            <div>
              <dt>GLB 模型</dt>
              <dd>{formatBytes(statusQuery.data.data.limits.maxGlbBytes)}</dd>
            </div>
            <div>
              <dt>物品累计</dt>
              <dd>{formatBytes(statusQuery.data.data.limits.maxItemTotalBytes)}</dd>
            </div>
            <div>
              <dt>JSON 请求体</dt>
              <dd>{formatBytes(statusQuery.data.data.limits.maxJsonRequestBytes)}</dd>
            </div>
          </dl>
        )}
      </section>

      <section className="panel" aria-labelledby="health-title">
        <h2 id="health-title">健康检查</h2>
        {liveQuery.isPending && <Skeleton label="正在读取健康状态…" rows={1} />}
        {liveQuery.data !== undefined && (
          <p>
            存活探针 <code>/health/live</code>：<span className="status-label">{liveQuery.data.data.status}</span>
          </p>
        )}
        {readyQuery.isPending && <Skeleton label="正在读取就绪状态…" rows={1} />}
        {readyQuery.data !== undefined && (
          <>
            <p>
              就绪探针 <code>/health/ready</code>：
              <span className="status-label">{readyQuery.data.status}</span>
            </p>
            <ul className="status-list">
              {readyQuery.data.checks.map((check) => (
                <li key={check.name}>
                  <span className="status-list__name">{check.name}</span>
                  <span className="status-label">{check.status}</span>
                </li>
              ))}
            </ul>
            <p className="empty-note">
              就绪检查只覆盖本机数据层（进程、数据目录、数据库、迁移）；云端供应商不可达不会使就绪失败。
            </p>
          </>
        )}
      </section>

      <section className="panel" aria-labelledby="deploy-title">
        <h2 id="deploy-title">部署边界</h2>
        <p>
          本服务默认监听回环地址；对外暴露需要位于显式受信的反向代理之后并由代理终止 TLS。
          本页不提供证书、TLS 或监听配置入口（MVP 不包含内置 TLS 监听）。
        </p>
      </section>
    </section>
  );
}
