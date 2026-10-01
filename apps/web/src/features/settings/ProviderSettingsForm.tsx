import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { describeError, isApiError } from "../../api/client";
import { fetchProviderSettings, saveProviderSettings, type ProviderSettingsData, type ProviderSettingsWrite, type ProviderView } from "../../api/endpoints";
import { Skeleton } from "../../components/Skeleton";

type Name = "tripo" | "manualAi";
type Edit = { baseUrl: string; model: string; keyAction: "keep" | "replace" | "clear"; apiKey: string; restore: boolean };
type Edits = Record<Name, Edit>;
const names: Name[] = ["tripo", "manualAi"];
const titles = { tripo: "Tripo", manualAi: "说明书 AI" };
const source = { web: "网页配置", deployment: "部署配置（含默认值）", default: "默认值", unconfigured: "未配置" };
const hasControl = (value: string) => [...value].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127);
function editor(view: ProviderView): Edit { return { baseUrl: view.baseUrl, model: view.model ?? "", keyAction: "keep", apiKey: "", restore: false }; }
function editors(data: ProviderSettingsData): Edits { return { tripo: editor(data.saved.tripo), manualAi: editor(data.saved.manualAi) }; }
export function validateProviderEdits(edits: Edits): Record<string, string> {
  const issues: Record<string, string> = {};
  for (const name of names) {
    const edit = edits[name];
    if (edit.restore) continue;
    const raw = edit.baseUrl.trim();
    try {
      const url = new URL(raw);
      if ([...raw].length > 2048 || /\s/u.test(raw) || hasControl(edit.baseUrl) || url.username || url.password || url.search || url.hash || !(url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)))) throw new Error();
    } catch { issues[`${name}.baseUrl`] = "请输入有效 HTTPS 地址；HTTP 仅允许明确的本机地址。不能含账号密码、查询参数或片段。"; }
    if ([...edit.model.trim()].length > 128 || hasControl(edit.model)) issues[`${name}.model`] = "模型最多 128 字符，不能含控制字符。";
    if (edit.keyAction === "replace" && (!edit.apiKey.trim() || [...edit.apiKey.trim()].length > 4096 || /\s/u.test(edit.apiKey.trim()) || hasControl(edit.apiKey.trim()))) issues[`${name}.apiKey`] = "新密钥不能为空、超过 4096 字符或包含空白、控制字符。";
  }
  return issues;
}

/** 只保护设置页；不更换全站路由。原生 beforeunload 与站内单一 dialog。 */
function useUnsavedChanges(dirty: boolean) {
  const navigate = useNavigate();
  const [leave, setLeave] = useState<(() => void) | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const bypass = useRef(false);
  const leavesPage = useRef(true);
  const requestLeave = useCallback((action: () => void, navigation = true) => { leavesPage.current = navigation; setLeave(() => action); }, []);
  useEffect(() => {
    if (leave !== null) { dialog.current?.showModal(); cancel.current?.focus(); }
    else dialog.current?.close();
  }, [leave]);
  useEffect(() => {
    if (!dirty) { bypass.current = false; return; }
    const index = Number(window.history.state?.idx ?? 0);
    let restoring = false;
    const beforeUnload = (event: BeforeUnloadEvent) => { if (!bypass.current) { event.preventDefault(); event.returnValue = ""; } };
    const click = (event: MouseEvent) => {
      if (bypass.current || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
      if (!(anchor instanceof HTMLAnchorElement) || anchor.target || anchor.hasAttribute("download")) return;
      const url = new URL(anchor.href);
      if (url.origin !== location.origin || `${url.pathname}${url.search}` === `${location.pathname}${location.search}`) return;
      event.preventDefault(); event.stopPropagation();
      requestLeave(() => navigate(`${url.pathname}${url.search}${url.hash}`));
    };
    const pop = () => {
      if (bypass.current) return;
      const target = Number(window.history.state?.idx ?? index);
      if (restoring) { restoring = false; return; }
      if (target === index) return;
      restoring = true;
      window.history.go(index - target);
      requestLeave(() => window.history.go(target - index));
    };
    // 在 Router 的冒泡监听之前截住 popstate；恢复原 history entry 不卸载表单。
    const capturePop = (event: PopStateEvent) => { if (!bypass.current) { event.stopImmediatePropagation(); pop(); } };
    window.addEventListener("beforeunload", beforeUnload);
    document.addEventListener("click", click, true);
    window.addEventListener("popstate", capturePop, true);
    return () => { window.removeEventListener("beforeunload", beforeUnload); document.removeEventListener("click", click, true); window.removeEventListener("popstate", capturePop, true); };
  }, [dirty, navigate, requestLeave]);
  const prompt = <dialog ref={dialog} className="settings-leave-dialog" aria-labelledby="settings-leave-title" onCancel={(event) => { event.preventDefault(); setLeave(null); }}>
    <h2 id="settings-leave-title">丢弃未保存修改？</h2>
    <p>有未保存的 API 配置，离开将丢弃这些修改及新输入的密钥。</p>
    <div className="form-actions"><button ref={cancel} type="button" onClick={() => setLeave(null)}>继续编辑</button><button type="button" onClick={() => { const action = leave; bypass.current = leavesPage.current; setLeave(null); action?.(); }}>{leavesPage.current ? "丢弃并离开" : "丢弃并重新加载"}</button></div>
  </dialog>;
  return { prompt, requestLeave };
}

export function ProviderSettingsForm() {
  const queryClient = useQueryClient();
  const [data, setData] = useState<ProviderSettingsData | null>(null);
  const [edits, setEdits] = useState<Edits | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const [readError, setReadError] = useState<unknown>(null);
  const [saveError, setSaveError] = useState<unknown>(null);
  const [fields, setFields] = useState<Record<string, string>>({});
  const pendingFocus = useRef<string | null>(null);
  const [notice, setNotice] = useState("");
  const [conflict, setConflict] = useState(false);
  const mounted = useRef(true);
  const dirty = data !== null && edits !== null && JSON.stringify(edits) !== JSON.stringify(editors(data));
  const { prompt, requestLeave } = useUnsavedChanges(dirty);
  const read = useCallback(async () => {
    setLoading(true); setReadError(null);
    try {
      const response = await fetchProviderSettings();
      if (mounted.current) { setData(response.data); setEdits(editors(response.data)); setConflict(false); setSaveError(null); setFields({}); setNotice(""); }
    } catch (error) { if (mounted.current) setReadError(error); }
    finally { if (mounted.current) setLoading(false); }
  }, []);
  useEffect(() => { mounted.current = true; void read(); return () => { mounted.current = false; }; }, [read]);
  function reload() { if (dirty) requestLeave(() => { void read(); }, false); else void read(); }
  function update(name: Name, patch: Partial<Edit>) { setEdits((old) => old && { ...old, [name]: { ...old[name], ...patch } }); setNotice(""); }
  function focusIssue(issues: Record<string, string>) { pendingFocus.current = Object.keys(issues)[0] ?? null; }
  useEffect(() => {
    if (!saving && pendingFocus.current !== null) {
      document.getElementById(`api-${pendingFocus.current}`)?.focus();
      pendingFocus.current = null;
    }
  }, [fields, saving]);
  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (data === null || edits === null || savingRef.current || conflict) return;
    const issues = validateProviderEdits(edits); setFields(issues); setSaveError(null); setNotice("");
    if (Object.keys(issues).length > 0) { focusIssue(issues); return; }
    const editBody = (edit: Edit): ProviderSettingsWrite["tripo"] => edit.restore ? { action: "restore" } : { action: "update", baseUrl: edit.baseUrl.trim(), model: edit.model.trim(), keyAction: edit.keyAction, ...(edit.keyAction === "replace" ? { apiKey: edit.apiKey.trim() } : {}) };
    savingRef.current = true; setSaving(true);
    try {
      const response = await saveProviderSettings({ revision: data.revision, tripo: editBody(edits.tripo), manualAi: editBody(edits.manualAi) });
      if (!mounted.current) return;
      setData(response.data); setEdits(editors(response.data)); setFields({});
      setNotice(response.data.revision === data.revision ? "配置未变化" : response.data.pending ? "已保存，重启服务后生效" : "当前运行配置已生效；此前的报价仍需重新获取并确认");
      void queryClient.invalidateQueries({ queryKey: ["settings", "status"] });
    } catch (error) {
      if (!mounted.current) return;
      setSaveError(error);
      if (isApiError(error)) {
        setConflict(error.status === 409 && error.code === "REVISION_CONFLICT");
        const details = error.details as { fields?: Array<{ field: string; message: string }> } | null;
        const serverIssues = Object.fromEntries((details?.fields ?? []).map((issue) => [issue.field, issue.message]));
        setFields(serverIssues); focusIssue(serverIssues);
      }
    } finally { savingRef.current = false; if (mounted.current) setSaving(false); }
  }
  return <section className="provider-settings" aria-labelledby="api-settings-title">
    <h2 id="api-settings-title">API 配置</h2>
    <p className="field-hint">通过网页新增或替换的 API 密钥会加密保存在服务端。环境变量提供的密钥仅在运行内存中使用，本应用不会将其写入文件。</p>
    {loading && <Skeleton label="正在读取 API 配置…" rows={3} />}
    {readError !== null && <div className="error-panel" role="alert"><p>无法读取 API 配置。{describeError(readError).message}</p><p>请求 ID：{describeError(readError).requestId ?? "暂不可用"}</p><button type="button" onClick={reload}>重新读取</button></div>}
    {data !== null && edits !== null && <form onSubmit={(event) => void save(event)} noValidate>
      <div className="api-config-status" role="status"><strong>{data.pending ? "已保存，重启服务后生效" : "当前运行配置已生效"}</strong><span>连接未验证</span><p>{dirty ? "有未保存修改" : "所有修改已保存"}</p></div>
      {data.pending && <aside className="api-restart-note"><p>新报价、确认与生成操作暂不可用；已有资料和任务记录仍可查看。</p><p>在运行服务的终端停止并使用原启动方式重新启动；使用进程管理器时重启对应服务，然后刷新此页。</p><button type="button" disabled={saving || loading} onClick={reload}>刷新状态</button></aside>}
      <fieldset className="api-form-fields" disabled={saving || loading}>
        {names.map((name) => {
          const edit = edits[name], active = data.active[name], saved = data.saved[name], title = titles[name];
          function field(key: "baseUrl" | "model" | "apiKey", label: string, type: string, hint: string) { const path = `${name}.${key}`; return <div className="form-field"><label htmlFor={`api-${path}`}>{label}</label><input id={`api-${path}`} type={type} value={key === "apiKey" ? undefined : edit[key]} ref={key === "apiKey" ? (element) => { if (element && element.value !== edit.apiKey) element.value = edit.apiKey; } : undefined} autoComplete={key === "apiKey" ? "new-password" : "off"} spellCheck={false} aria-invalid={fields[path] !== undefined} aria-describedby={`api-${path}-hint${fields[path] ? ` api-${path}-error` : ""}`} onChange={(event) => update(name, { [key]: event.target.value })} /><p id={`api-${path}-hint`} className="field-hint">{hint}</p>{fields[path] && <p id={`api-${path}-error`} className="field-error">{fields[path]}</p>}</div>; }
          return <section className="panel api-provider-card" key={name} aria-labelledby={`api-${name}-title`}>
            <div className="api-card-heading"><h3 id={`api-${name}-title`}>{title}{name === "tripo" ? " · 模型生成" : ""}</h3><span>{name === "tripo" ? "Tripo v3" : "Responses 协议"}</span></div>
            <h4>当前运行配置</h4><dl className="api-current-values"><div><dt>地址 · {source[active.baseUrlSource]}</dt><dd>{active.baseUrl}</dd></div><div><dt>模型 · {source[active.modelSource]}</dt><dd>{active.model ?? "未配置"}</dd></div><div><dt>密钥 · {source[active.keySource]}</dt><dd>{active.keyConfigured ? "已配置" : "未配置"}</dd></div></dl>
            <h4>下次启动配置</h4>
            {edit.restore ? <div className="api-restore-note"><p>保存后恢复部署配置</p><p>将撤销本供应商的网页地址、模型和密钥覆盖；另一家不受影响；重启后使用部署配置。</p><button type="button" onClick={() => { update(name, editor(saved)); setNotice("已取消恢复；如需替换密钥，请重新输入"); }}>取消恢复</button></div> : <>
              <div className="api-editor-grid">{field("baseUrl", `${title} Base URL`, "url", name === "tripo" ? "填写兼容 Tripo v3 的基础地址，不是某个生成接口的完整地址。" : "填写兼容 Responses 的基础地址，不要填写完整的 /responses 接口地址。")}{field("model", `${title} 模型`, "text", "可留空；生成仍要求模型与服务端价格目录匹配。保存不会验证模型或余额。")}</div>
              <p className="field-hint">已保存来源：地址 {source[saved.baseUrlSource]}；模型 {source[saved.modelSource]}；密钥 {source[saved.keySource]}（{saved.keyConfigured ? "已配置" : "未配置"}）。</p>
              <fieldset className="api-key-options"><legend>{title} 密钥操作</legend>{(["keep", "replace", "clear"] as const).map((action) => <label key={action}><input type="radio" name={`api-${name}-keyAction`} value={action} checked={edit.keyAction === action} onChange={() => update(name, { keyAction: action, apiKey: "" })} />{({ keep: "保留现有", replace: "替换", clear: "清除" })[action]}</label>)}</fieldset>
              {edit.keyAction === "keep" && <p className="field-hint">保留已保存的密钥选择与来源。{!saved.keyConfigured && "当前没有密钥，保留不会补充密钥。"}</p>}
              {edit.keyAction === "replace" && field("apiKey", `新的${title === "Tripo" ? " Tripo " : title}密钥`, "password", "新密钥将在服务端加密保存，已保存密钥不会回显。")}
              {edit.keyAction === "clear" && <p className="api-impact">保存后将不使用该供应商的密钥，也不会自动使用部署配置中的密钥；重启后该供应商不能生成。</p>}
              {(!saved.keyConfigured || !saved.model) && <p className="status-note">已保存配置缺项：{[!saved.keyConfigured && "密钥", !saved.model && "模型"].filter(Boolean).join("、")}。可保存不完整配置；生成需先补齐。</p>}
              <button type="button" className="button--secondary" onClick={() => update(name, { restore: true, apiKey: "", keyAction: "keep" })}>恢复部署配置</button>
            </>}
          </section>;
        })}
      </fieldset>
      <p className="field-hint">使用 HTTPS；本机 localhost、127.0.0.1 或 [::1] 可使用 HTTP。地址不能带账号密码、查询参数或片段。网页设置优先；恢复部署配置后使用部署侧设置。</p>
      {names.filter((name) => edits[name].restore || edits[name].keyAction === "clear").map((name) => <p className="api-impact" key={name}>{titles[name]}：{edits[name].restore ? "本次保存将撤销全部网页覆盖，重启后使用部署配置。" : "本次保存将清除密钥并屏蔽部署密钥，重启后不能生成。"}</p>)}
      {Object.keys(fields).length > 0 && <div role="alert" className="error-panel"><p>请修正以下字段：</p><ul>{Object.entries(fields).map(([path, message]) => <li key={path}><a href={`#api-${path}`} onClick={() => document.getElementById(`api-${path}`)?.focus()}>{message}</a></li>)}</ul></div>}
      {saveError !== null && <div className="error-panel" role="alert"><p>{conflict ? "配置已在其他页面更新，你的编辑尚未保存。" : "未能保存配置。"}{describeError(saveError).message}</p><p>请求 ID：{describeError(saveError).requestId ?? "暂不可用"}</p>{conflict ? <><p>重新加载将丢弃本页编辑和新密钥。</p><button type="button" onClick={reload}>重新加载已保存配置</button></> : <><p>请求结果可通过重新读取核对；未保存的输入仍保留在当前页。</p><button type="button" onClick={reload}>重新读取配置</button><Link to="/jobs">前往任务中心</Link></>}</div>}
      <div className="api-save-row"><button type="submit" className="button--primary" disabled={saving || loading || conflict || !dirty} aria-busy={saving}>{saving ? "正在保存…" : "保存配置"}</button><span role="status">{saving ? "正在保存 API 配置…" : notice}</span></div>
      <p className="field-hint">备份与导出不包含 API 密钥；迁移服务后需重新配置。</p>
      <p className="field-hint">加密存储与部署迁移方法见项目文档《在网页中配置 API》。</p>
    </form>}
    {prompt}
  </section>;
}
