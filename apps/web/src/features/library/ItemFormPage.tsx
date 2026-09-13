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
import { useNavigate, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { ConflictNotice } from "../../components/ConflictNotice";
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
  useCreateItem,
  useItemDetail,
  usePatchItem,
  type PatchItemVariables,
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
    return <EditItemForm itemId={itemId ?? ""} />;
  }
  return <CreateItemForm />;
}

function CreateItemForm() {
  const createMutation = useCreateItem();
  const navigate = useNavigate();
  const notify = useNotify();
  const [values, setValues] = useState<ItemFormValues>(EMPTY_VALUES);
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [formError, setFormError] = useState<string | null>(null);
  const [requestId, setRequestId] = useState<string | null>(null);

  async function onSubmit() {
    setFieldErrors([]);
    setFormError(null);
    setRequestId(null);
    try {
      const created = await createMutation.mutateAsync(toCreateRequest(values));
      notify(`已创建物品「${created.data.name}」`);
      navigate(`/items/${created.data.id}`);
    } catch (error) {
      const errors = isApiError(error) ? readFieldErrors(error.details) : [];
      const info = describeError(error);
      setFieldErrors(errors);
      setFormError(info.message);
      setRequestId(info.requestId);
    }
  }

  return (
    <section className="page item-form-page" aria-labelledby="item-form-title">
      {/* 向导第 1 步：物品还不存在，后续步骤不可导航（WizardSteps 渲染为不可用项并说明）。 */}
      <WizardSteps currentSegment="edit" itemId={null} />
      <h1 id="item-form-title">新建物品</h1>
      <p className="page__lead">填写物品的基本信息；下一步再上传说明书原件与多视图照片。</p>
      <ItemForm
        values={values}
        onChange={setValues}
        onSubmit={onSubmit}
        pending={createMutation.isPending}
        submitLabel="创建并继续"
        fieldErrors={fieldErrors}
        formError={formError}
        requestId={requestId}
      />
    </section>
  );
}

function EditItemForm({ itemId }: { itemId: string }) {
  const itemQuery = useItemDetail(itemId);
  const patchMutation = usePatchItem();
  const navigate = useNavigate();
  const notify = useNotify();
  const [values, setValues] = useState<ItemFormValues | null>(null);
  const [etag, setEtag] = useState<string | null>(null);
  const [conflict, setConflict] = useState<{ currentRevision: number | null } | null>(null);
  const [refreshed, setRefreshed] = useState<number | null>(null);
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [formError, setFormError] = useState<string | null>(null);
  const [requestId, setRequestId] = useState<string | null>(null);

  const item = itemQuery.data?.data;
  const effectiveEtag = etag ?? itemQuery.data?.etag ?? null;

  async function onSubmit(nextValues: ItemFormValues) {
    setFieldErrors([]);
    setFormError(null);
    setRequestId(null);
    if (effectiveEtag === null) {
      setConflict({ currentRevision: null });
      return;
    }
    const variables: PatchItemVariables = {
      itemId,
      body: toPatchRequest(nextValues),
      ifMatch: effectiveEtag,
    };
    try {
      const result = await patchMutation.mutateAsync(variables);
      setEtag(result.etag);
      setRefreshed(null);
      notify(`已保存「${result.data.name}」`);
      navigate(`/items/${itemId}`);
    } catch (error) {
      if (isApiError(error) && (error.status === 412 || error.status === 428)) {
        // 不自动覆盖、不丢弃输入：只提示并等待用户刷新（UI-008）。
        setConflict({ currentRevision: readCurrentRevision(error.details) });
        return;
      }
      const errors = isApiError(error) ? readFieldErrors(error.details) : [];
      const info = describeError(error);
      setFieldErrors(errors);
      setFormError(info.message);
      setRequestId(info.requestId);
    }
  }

  async function refreshAfterConflict() {
    const result = await itemQuery.refetch();
    const refreshedItem = result.data?.data;
    setEtag(result.data?.etag ?? null);
    setRefreshed(refreshedItem?.revision ?? null);
    setConflict(null);
  }

  if (itemQuery.isPending) {
    return (
      <section className="page">
        <Skeleton label="正在读取物品…" rows={4} />
      </section>
    );
  }

  if (item === undefined) {
    const info = describeError(itemQuery.error);
    return (
      <section className="page">
        <div className="error-panel" role="alert">
          <h1>无法读取物品</h1>
          <p>{info.message}</p>
          {info.requestId !== null && (
            <p>
              诊断请求 ID：<code>{info.requestId}</code>
            </p>
          )}
          <button type="button" onClick={() => void itemQuery.refetch()}>
            重试
          </button>
        </div>
      </section>
    );
  }

  const currentValues = values ?? toFormValues(item);

  return (
    <section className="page item-form-page" aria-labelledby="item-form-title">
      <h1 id="item-form-title">编辑物品</h1>
      <p className="page__lead">
        当前版本 r{item.revision}；保存时按该版本提交，若他人已更新会提示刷新而不是覆盖。
      </p>
      {/* UI-021：修改不影响已开始任务使用的冻结快照。 */}
      <JobSnapshotNotice itemId={itemId} />
      {conflict !== null && (
        <ConflictNotice
          currentRevision={conflict.currentRevision}
          refreshing={itemQuery.isFetching}
          onRefresh={() => void refreshAfterConflict()}
        />
      )}
      {refreshed !== null && conflict === null && (
        <p className="status-note" role="status">
          {`已刷新到服务端最新版本（r${refreshed}）；请确认表单内容后重新提交。`}
        </p>
      )}
      <ItemForm
        values={currentValues}
        onChange={setValues}
        onSubmit={onSubmit}
        pending={patchMutation.isPending}
        submitLabel="保存"
        fieldErrors={fieldErrors}
        formError={formError}
        requestId={requestId}
        disabled={conflict !== null}
      />
    </section>
  );
}

interface ItemFormProps {
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

      {bodyErrors.length > 0 && (
        <ul className="form-errors__plain">
          {bodyErrors.map((error) => (
            <li key={error.field}>{error.message}</li>
          ))}
        </ul>
      )}

      <p className="item-form__note">
        物品不保存来源链接：出处链接在「绑定说明书原件」时记录（仅作出处，服务器不会访问该地址）。
      </p>

      <button type="submit" className="button-primary" disabled={missingRequired || pending || disabled}>
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
