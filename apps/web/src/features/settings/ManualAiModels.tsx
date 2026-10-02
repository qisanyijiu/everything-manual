import { useEffect, useRef, useState } from "react";
import { describeError } from "../../api/client";
import { readManualAiModels, type ManualAiModels as Models } from "../../api/endpoints";
import { isSuspectedCredentialModel } from "./model-guard";

export function ManualAiModels({ keyConfigured, pending, onSelect }: {
  keyConfigured: boolean;
  pending: boolean;
  onSelect: (model: string) => void;
}) {
  const [models, setModels] = useState<Models | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const busy = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function read() {
    if (busy.current || !keyConfigured) return;
    busy.current = true; setLoading(true); setError(null); setModels(null);
    try {
      const response = await readManualAiModels();
      // Fail closed if a stale/misconfigured server does not honor the safe IDs-only contract.
      if (!Array.isArray(response.data) || response.data.length > 1_000 || response.data.some((id) => typeof id !== "string" || !id || [...id].length > 128 || /[\s\p{Cc}]/u.test(id) || isSuspectedCredentialModel(id))) {
        throw new Error("模型列表格式无效，未显示任何返回值。");
      }
      if (mounted.current) setModels(response.data);
    } catch (cause) { if (mounted.current) setError(cause); }
    finally { busy.current = false; if (mounted.current) setLoading(false); }
  }
  return <div className="form-field">
    <p id="manual-ai-models-hint" className="field-hint">{pending ? "存在待重启配置；本次仍从当前已生效的地址和密钥读取。" : "使用当前已生效的地址和密钥读取。"}页面内未保存的输入不参与读取；选择后只填入模型字段，仍需保存并重启。模型列表不代表 Responses 能力或价格目录已匹配。</p>
    <button type="button" aria-describedby="manual-ai-models-hint" disabled={loading || !keyConfigured} aria-busy={loading} onClick={() => void read()}>{loading ? "正在读取模型…" : "读取可用模型"}</button>
    {!keyConfigured && <p className="field-hint">当前生效配置没有密钥，请保存密钥并重启服务后再读取。</p>}
    {models !== null && (models.length === 0 ? <p role="status">服务未返回可选模型；可以手动填写模型名称。</p> : <>
      <label htmlFor="manual-ai-available-models">可用的说明书 AI 模型</label>
      <select id="manual-ai-available-models" style={{ display: "block", width: "100%", minWidth: 0, minHeight: 44 }} value="" aria-describedby="manual-ai-models-hint" onChange={(event) => { if (event.target.value) onSelect(event.target.value); }}>
        <option value="" disabled>选择后填入模型字段</option>
        {models.map((id) => <option key={id} value={id}>{id}</option>)}
      </select>
      <p role="status">已读取 {models.length} 个模型。请选择需要填入的模型。</p>
    </>)}
    {error !== null && <div className="error-panel" role="alert"><p>无法读取可用模型。{describeError(error).message}</p><p>请求 ID：{describeError(error).requestId ?? "暂不可用"}</p><p>可再次点击读取，或手动填写服务支持的模型名称。</p></div>}
  </div>;
}
