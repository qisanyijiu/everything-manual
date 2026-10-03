/**
 * 新建/编辑物品表单（PRD §6.2 UI-006、UI-008；`/items/new`、`/items/:itemId/edit`）。
 *
 * - 字段：名称、品牌、准确型号、变体/配置；**没有物品级来源链接字段**
 *   （A-16/D-1：出处链接只在绑定说明书原件时写入 `document.sourceUrl`）；
 * - 新建 `POST /items`（201）；编辑 `PATCH /items/{id}`（If-Match，ETag 原样回传）；
 * - 422 → `details.fields` 摘要锚点 + 字段级 `aria-describedby`，焦点移到第一个错误字段；
 * - 412/428 → UI-008：显示 `details.currentRevision` 与「刷新后重试」，不自动覆盖、
 *   不丢表单内容、刷新前禁用提交；
 * - 必填为空时禁用保存（UI-006 禁用态）。
 */

import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { useQueryClient } from "@tanstack/react-query";
import { useMemoryEdit, usePageWork, useWorkProtection } from "../shell/work-protection";
import { getItem, listItems } from "../../api/endpoints";
import {
  fieldInputId,
  FormErrorSummary,
  readCurrentRevision,
  readFieldErrors,
  TextField,
  type FieldError,
} from "../../components/form";
import { Skeleton } from "../../components/Skeleton";
import { useNotify } from "../../components/notifications";
import { WizardSteps } from "../import/WizardSteps";
import { JobSnapshotNotice } from "./JobSnapshotNotice";
import type { ItemCreateRequest, ItemDto, ItemPatchRequest } from "../../api/endpoints";
import {
  itemKeys,
  useCreateItem,
  useItemDetail,
  usePatchItem,
} from "./items";

export interface ItemFormValues {
  readonly name: string;
  readonly model: string;
  readonly brand: string;
  readonly variant: string;
}

const EMPTY_VALUES: ItemFormValues = { name: "", model: "", brand: "", variant: "" };

export function ItemFormPage({ mode }: { mode: "create" | "edit" }) {
  const { itemId } = useParams();
  if (mode === "edit") {
    return <EditItemForm key={itemId} itemId={itemId ?? ""} />;
  }
  return <CreateItemForm />;
}

type SaveState = "clean" | "dirty" | "error" | "uncertain" | "conflict";
interface ItemEdit {
  values: ItemFormValues | null;
  etag: string | null;
  base?: ItemFormValues;
  status: SaveState;
  error: string | null;
  requestId: string | null;
  revision: number | null;
}
const EMPTY_EDIT: ItemEdit = { values: null, etag: null, status: "clean", error: null, requestId: null, revision: null };
function unknownResult(error: unknown) { return !isApiError(error) || error.status >= 500; }
function saveStatus(pending: boolean, state: SaveState, dirty: boolean) {
  return pending ? "正在保存…" : state === "error" || state === "uncertain" || state === "conflict" ? "保存失败，修改仍在本页" : dirty ? "有未保存修改" : "尚无未保存修改";
}
function useLiveForm() {
  const mounted = useRef(true), lock = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  return { mounted, lock };
}
function CreateItemForm() {
  const createMutation = useCreateItem(), navigate = useNavigate(), notify = useNotify();
  const { bypass } = useWorkProtection();
  const [edit, setEdit, clear] = useMemoryEdit<ItemEdit>("item:new", EMPTY_EDIT);
  const { mounted, lock } = useLiveForm();
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [checking, setChecking] = useState(false);
  const [matches, setMatches] = useState<ItemDto[] | null>(null);
  const values = edit.values ?? EMPTY_VALUES;
  const dirty = JSON.stringify(values) !== JSON.stringify(EMPTY_VALUES);
  usePageWork({ active: dirty || createMutation.isPending, message: `离开将丢弃本页未保存的物品信息。已保存资料保留。${createMutation.isPending || edit.status === "uncertain" ? "已提交的请求可能已经完成，请先核对结果。" : ""}`, discard: clear });
  async function onSubmit() {
    if (lock.current || edit.status === "uncertain") return;
    lock.current = true; setFieldErrors([]);
    // Before sending, mark uncertainty in memory so even an intervening 401 cannot enable a blind POST.
    setEdit({ ...edit, values, status: "uncertain", error: null, requestId: null });
    try {
      const created = await createMutation.mutateAsync(toCreateRequest(values));
      if (!mounted.current) return;
      clear(); notify(`已保存，已创建物品「${created.data.name}」`);
      bypass(() => navigate(`/items/${created.data.id}/import/document`));
    } catch (error) {
      const info = describeError(error);
      setEdit(previous => ({ ...previous, status: unknownResult(error) ? "uncertain" : "error", error: info.message, requestId: info.requestId }));
      if (mounted.current) setFieldErrors(isApiError(error) ? readFieldErrors(error.details) : []);
    } finally { lock.current = false; }
  }
  async function checkCreation() {
    if (checking) return;
    setChecking(true);
    try { const result = await listItems({ q: values.name.trim().slice(0, 200), limit: 20 }); if (mounted.current) setMatches(result.items); }
    catch (error) { setEdit(previous => ({ ...previous, error: `核对失败：${describeError(error).message}` })); }
    finally { if (mounted.current) setChecking(false); }
  }
  return <section className="page item-form-page" aria-labelledby="item-form-title">
    <WizardSteps currentSegment="edit" itemId={null} /><h1 id="item-form-title">新建物品</h1>
    <p className="page__lead">填写物品的基本信息；下一步再上传说明书原件与多视图照片。</p>
    {edit.status === "uncertain" && !createMutation.isPending && <div className="error-panel" role="alert"><p>创建结果未知，请先读取核对。未收到响应不代表未创建，不会自动重复提交。</p><button type="button" disabled={checking} onClick={() => void checkCreation()}>核对创建结果</button>
      {matches !== null && <><p>以下是名称匹配的已保存物品；名称相同或未找到都不能证明本次请求的结果。请打开核对，暂不重复创建。</p><ul>{matches.map(item => <li key={item.id}><Link to={`/items/${item.id}`}>{item.name} · {item.model}</Link></li>)}</ul><Link to="/">查看资料库</Link></>}
    </div>}
    <ItemForm values={values} onChange={next => setEdit(previous => ({ ...previous, values: next, status: previous.status === "uncertain" ? "uncertain" : "dirty" }))} onSubmit={onSubmit}
      pending={createMutation.isPending} submitLabel="创建并继续" fieldErrors={fieldErrors} formError={edit.error} requestId={edit.requestId} disabled={edit.status === "uncertain"}
      status={saveStatus(createMutation.isPending, edit.status, dirty)} />
  </section>;
}

function EditItemForm({ itemId }: { itemId: string }) {
  const itemQuery = useItemDetail(itemId), patchMutation = usePatchItem(), queryClient = useQueryClient();
  const navigate = useNavigate(), notify = useNotify();
  const { bypass, request } = useWorkProtection();
  const [edit, setEdit, clear] = useMemoryEdit<ItemEdit>(`item:${itemId}`, EMPTY_EDIT);
  const { mounted, lock } = useLiveForm();
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [checking, setChecking] = useState(false);
  const [latest, setLatest] = useState<ItemDto | null>(null);
  const item = itemQuery.data?.data;
  const currentValues = edit.values ?? (item ? toFormValues(item) : EMPTY_VALUES);
  const effectiveEtag = edit.etag ?? itemQuery.data?.etag ?? null;
  const dirty = edit.values !== null && JSON.stringify(edit.values) !== JSON.stringify(edit.base ?? (item ? toFormValues(item) : EMPTY_VALUES));
  const blocked = edit.status === "conflict" || edit.status === "uncertain";
  usePageWork({ active: dirty || patchMutation.isPending, message: `离开将丢弃本页未保存的物品修改。已保存资料保留。${patchMutation.isPending || edit.status === "uncertain" ? "已提交的请求可能已经完成，请核对后再操作。" : ""}`, discard: clear });
  async function onSubmit(nextValues: ItemFormValues) {
    if (lock.current || blocked || effectiveEtag === null) return;
    lock.current = true; setFieldErrors([]); setLatest(null);
    setEdit({ ...edit, values: nextValues, etag: effectiveEtag, status: "uncertain", error: null, requestId: null });
    try {
      await patchMutation.mutateAsync({ itemId, body: toPatchRequest(nextValues), ifMatch: effectiveEtag });
      if (!mounted.current) return;
      clear(); notify("已保存物品修改"); bypass(() => navigate(`/items/${itemId}`));
    } catch (error) {
      const info = describeError(error), conflict = isApiError(error) && (error.status === 412 || error.status === 428);
      setEdit(previous => ({ ...previous, status: conflict ? "conflict" : unknownResult(error) ? "uncertain" : "error", error: info.message, requestId: info.requestId, revision: conflict && isApiError(error) ? readCurrentRevision(error.details) : null }));
      if (mounted.current) setFieldErrors(isApiError(error) ? readFieldErrors(error.details) : []);
    } finally { lock.current = false; }
  }
  async function readLatest(discard = false) {
    if (checking) return;
    setChecking(true);
    try {
      const result = await getItem(itemId);
      if (!mounted.current) return;
      if (discard) {
        queryClient.setQueryData(itemKeys.detail(itemId), result);
        setEdit({ ...EMPTY_EDIT, etag: result.etag }); setLatest(null); setFieldErrors([]);
        notify("已加载最新版本，本页未保存修改已丢弃");
      } else setLatest(result.data);
    } catch (error) { setEdit(previous => ({ ...previous, error: `读取失败，修改仍在本页。${describeError(error).message}` })); }
    finally { if (mounted.current) setChecking(false); }
  }
  if (itemQuery.isPending && !edit.values) return <section className="page"><Skeleton label="正在读取物品…" rows={4} /></section>;
  if (!item && !edit.values) return <section className="page"><div className="error-panel" role="alert"><h1>无法读取物品</h1><p>{describeError(itemQuery.error).message}</p><button type="button" onClick={() => void itemQuery.refetch()}>重试</button></div></section>;
  return <section className="page item-form-page" aria-labelledby="item-form-title">
    <h1 id="item-form-title">编辑物品</h1><p className="page__lead">保存时核对编辑开始时的版本；若其他操作已更新，不会自动覆盖。</p><JobSnapshotNotice itemId={itemId} />
    {blocked && <div className="conflict-notice" role="alert"><p>{edit.status === "conflict" ? `该内容已被其他操作更新（当前 r${edit.revision ?? "待核对"}）` : "保存结果未知，请先读取核对。"}</p><p>你的修改仍在本页。核对只读取，不会替换输入或换用新版本提交。</p><button type="button" disabled={checking} onClick={() => void readLatest()}>核对最新版本</button>
      {latest && <section aria-label="服务器最新物品"><h2>服务器最新版本 r{latest.revision}</h2><p>{latest.name} · {latest.model} · {latest.brand} · {latest.variant}</p></section>}
      <button type="button" disabled={checking} onClick={() => request(() => { void readLatest(true); }, { message: "将丢弃本页未保存的物品修改；读取成功后加载服务器最新版本。读取失败仍保留输入。", accept: "丢弃本页修改并加载最新版本" })}>丢弃本页修改并加载最新版本</button>
    </div>}
    <ItemForm values={currentValues} onChange={values => setEdit(previous => ({ ...previous, values, base: previous.base ?? (item ? toFormValues(item) : EMPTY_VALUES), etag: effectiveEtag, status: blocked ? previous.status : "dirty" }))} onSubmit={onSubmit}
      pending={patchMutation.isPending} submitLabel="保存" fieldErrors={fieldErrors} formError={edit.error} requestId={edit.requestId} disabled={blocked || checking || !effectiveEtag}
      status={saveStatus(patchMutation.isPending, edit.status, dirty)} />
  </section>;
}

interface ItemFormProps {
  readonly status: string;
  readonly values: ItemFormValues;
  readonly onChange: (values: ItemFormValues) => void;
  readonly onSubmit: (values: ItemFormValues) => void | Promise<void>;
  readonly pending: boolean;
  readonly submitLabel: string;
  readonly fieldErrors: readonly FieldError[];
  readonly formError: string | null;
  readonly requestId: string | null;
  readonly disabled?: boolean;
}

function ItemForm({
  values,
  onChange,
  onSubmit,
  pending,
  submitLabel,
  fieldErrors,
  formError,
  requestId,
  disabled = false,
  status,
}: ItemFormProps) {
  const summaryRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (fieldErrors.length === 0) {
      return;
    }
    const first = fieldErrors[0];
    if (first === undefined) {
      return;
    }
    const target = document.getElementById(fieldInputId(first.field));
    if (target !== null) {
      target.focus();
    } else {
      summaryRef.current?.focus();
    }
  }, [fieldErrors]);

  const errorOf = (field: string): string | null =>
    fieldErrors.find((error) => error.field === field)?.message ?? null;
  const bodyErrors = fieldErrors.filter((error) => error.field === "body");
  const missingRequired = values.name.trim() === "" || values.model.trim() === "";

  return (
    <form
      className="item-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        if (!missingRequired && !pending && !disabled) {
          void onSubmit(values);
        }
      }}
    >
      <FormErrorSummary id="item-form-errors" errors={fieldErrors} summaryRef={summaryRef} />

      {formError !== null && fieldErrors.length === 0 && (
        <div className="form-errors" role="alert">
          <h2 className="form-errors__title">保存失败</h2>
          <p>{formError}</p>
          {requestId !== null && (
            <p>
              诊断请求 ID：<code>{requestId}</code>
            </p>
          )}
        </div>
      )}

      <fieldset className="item-form-fields" disabled={pending}>
      <TextField
        field="name"
        label="名称"
        required
        value={values.name}
        onChange={(name) => onChange({ ...values, name })}
        error={errorOf("name")}
      />
      <TextField
        field="model"
        label="准确型号"
        required
        value={values.model}
        onChange={(model) => onChange({ ...values, model })}
        error={errorOf("model")}
      />
      <TextField
        field="brand"
        label="品牌"
        value={values.brand}
        onChange={(brand) => onChange({ ...values, brand })}
        error={errorOf("brand")}
      />
      <TextField
        field="variant"
        label="变体/配置"
        hint="例如同一型号下的不同镜头组合；同型号不同配置允许并存。"
        value={values.variant}
        onChange={(variant) => onChange({ ...values, variant })}
        error={errorOf("variant")}
      />

      </fieldset>
      {bodyErrors.length > 0 && (
        <ul className="form-errors__plain">
          {bodyErrors.map((error) => (
            <li key={error.field}>{error.message}</li>
          ))}
        </ul>
      )}

      <p className="item-form__note">
        说明书的出处链接，可在下一步上传原件时填写。
      </p>

      <p id="item-save-status" role="status" aria-live="polite">{status}</p>
      <button aria-describedby="item-save-status" type="submit" className="button-primary" disabled={missingRequired || pending || disabled}>
        {pending ? "保存中…" : submitLabel}
      </button>
    </form>
  );
}

export function toFormValues(item: ItemDto): ItemFormValues {
  return {
    name: item.name,
    model: item.model,
    brand: item.brand ?? "",
    variant: item.variant ?? "",
  };
}

function toCreateRequest(values: ItemFormValues): ItemCreateRequest {
  return {
    name: values.name.trim(),
    model: values.model.trim(),
    brand: values.brand.trim() === "" ? null : values.brand.trim(),
    variant: values.variant.trim() === "" ? null : values.variant.trim(),
  };
}

function toPatchRequest(values: ItemFormValues): ItemPatchRequest {
  return {
    name: values.name.trim(),
    model: values.model.trim(),
    brand: values.brand.trim() === "" ? null : values.brand.trim(),
    variant: values.variant.trim() === "" ? null : values.variant.trim(),
  };
}
