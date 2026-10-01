/**
 * 设置与状态（PRD §6.2 UI-003；`GET /settings/status` + `/health/live` + `/health/ready`）。
 *
 * API 配置允许管理员编辑地址、模型与密钥动作，受服务端认证/CSRF保护，重启生效。
 * 当前/已保存/本地编辑分别表达；密钥仅瞬时输入，不回显、不持久化到浏览器。
 * 输入限制、本机健康与部署边界保持独立读取；没有连接测试或网页重启。
 */

import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { fetchHealthLive, fetchReadiness, fetchSettingsStatus } from "../../api/endpoints";
import { Skeleton } from "../../components/Skeleton";
import { formatBytes } from "../../lib/format";
import { ProviderSettingsForm } from "./ProviderSettingsForm";

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
      <p className="eyebrow">WORKSPACE SETTINGS</p>
      <h1 id="settings-title">设置与状态</h1>
      <p className="page__lead">
        配置 Tripo 与说明书 AI。保存后需重启服务生效；保存不会验证连接或发起生成。
      </p>

      <ProviderSettingsForm />
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
                {statusQuery.data.data.providerConfigPending ? "待重启，生成暂停" : statusQuery.data.data.capabilities.generation ? "基础配置齐备，仍需报价校验" : "基础配置缺项"}
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
