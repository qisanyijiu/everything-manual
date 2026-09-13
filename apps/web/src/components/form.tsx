/**
 * 表单基础件（PRD §6.1.5 / §6.2 UI-065）：
 * 错误摘要（锚点到字段）+ 字段级 `aria-describedby` 关联；提交失败后焦点移到第一个错误字段
 * （登录页按 UI-001 聚焦错误摘要，由调用方决定聚焦目标）。
 *
 * 服务端错误形态（T07/ADR-016 第 4 条）：
 * - `details.fields = [{field, message}]`，字段名为线上 camelCase，请求体整体问题用 `body`；
 * - `details.currentRevision`（412）、`details.reason`（T06/T07 惯例）。
 */

import type { ReactNode, RefObject } from "react";

export interface FieldError {
  readonly field: string;
  readonly message: string;
}

const FIELD_LABELS: Record<string, string> = {
  name: "名称",
  model: "型号",
  brand: "品牌",
  variant: "配置",
  body: "请求内容",
};

export function fieldLabel(field: string): string {
  return FIELD_LABELS[field] ?? field;
}

/** 解析 `details.fields`；结构不符时返回空数组（调用方给通用兜底文案）。 */
export function readFieldErrors(details: unknown): FieldError[] {
  if (typeof details !== "object" || details === null) {
    return [];
  }
  const fields = (details as { fields?: unknown }).fields;
  if (!Array.isArray(fields)) {
    return [];
  }
  const result: FieldError[] = [];
  for (const entry of fields) {
    if (typeof entry !== "object" || entry === null) {
      continue;
    }
    const { field, message } = entry as { field?: unknown; message?: unknown };
    if (typeof field === "string" && typeof message === "string") {
      result.push({ field, message });
    }
  }
  return result;
}

/** 解析 412 的 `details.currentRevision`（缺失或非正整数时返回 null）。 */
export function readCurrentRevision(details: unknown): number | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  const value = (details as { currentRevision?: unknown }).currentRevision;
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

/** 读取 `details.reason`（T06/T07 的机器可读原因，如 `insufficientStorage`）。 */
export function readReason(details: unknown): string | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  const reason = (details as { reason?: unknown }).reason;
  return typeof reason === "string" ? reason : null;
}

export function fieldInputId(field: string): string {
  return `field-${field}`;
}

export interface TextFieldProps {
  readonly field: string;
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly required?: boolean;
  readonly error?: string | null;
  readonly hint?: ReactNode;
  readonly type?: "text" | "password";
  readonly autoComplete?: string;
  readonly inputRef?: RefObject<HTMLInputElement | null>;
}

/** 单行文本/密码输入：标签、必填标记、错误与说明的关联一次到位。 */
export function TextField({
  field,
  label,
  value,
  onChange,
  required = false,
  error = null,
  hint,
  type = "text",
  autoComplete,
  inputRef,
}: TextFieldProps) {
  const inputId = fieldInputId(field);
  const hintId = `${inputId}-hint`;
  const errorId = `${inputId}-error`;
  const describedBy = [hint !== undefined ? hintId : null, error !== null ? errorId : null]
    .filter((id): id is string => id !== null)
    .join(" ");

  return (
    <div className="field">
      <label className="field__label" htmlFor={inputId}>
        {label}
        {required && (
          <span className="field__required" aria-hidden="true">
            {" "}
            *
          </span>
        )}
        {required && <span className="visually-hidden">（必填）</span>}
      </label>
      <input
        id={inputId}
        className="field__input"
        type={type}
        value={value}
        required={required}
        autoComplete={autoComplete}
        ref={inputRef}
        aria-invalid={error !== null ? true : undefined}
        aria-describedby={describedBy === "" ? undefined : describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint !== undefined && (
        <p className="field__hint" id={hintId}>
          {hint}
        </p>
      )}
      {error !== null && (
        <p className="field__error" id={errorId}>
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * 错误摘要：`role="alert"` + 锚点到字段；`tabIndex={-1}` 让调用方能把焦点移到摘要本身。
 * 点击锚点把焦点交给字段（键盘/鼠标一致）。
 */
export function FormErrorSummary({
  id,
  errors,
  title = "请修正以下问题",
  summaryRef,
}: {
  id: string;
  errors: readonly FieldError[];
  title?: string;
  summaryRef?: RefObject<HTMLDivElement | null>;
}) {
  if (errors.length === 0) {
    return null;
  }
  return (
    <div className="form-errors" role="alert" id={id} tabIndex={-1} ref={summaryRef}>
      <h2 className="form-errors__title">{title}</h2>
      <ul className="form-errors__list">
        {errors.map((error) => (
          <li key={error.field}>
            <a
              href={`#${fieldInputId(error.field)}`}
              onClick={(event) => {
                const target = document.getElementById(fieldInputId(error.field));
                if (target !== null) {
                  event.preventDefault();
                  target.focus();
                }
              }}
            >
              {fieldLabel(error.field)}：{error.message}
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}
