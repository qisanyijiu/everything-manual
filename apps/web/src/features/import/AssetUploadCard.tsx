/**
 * 共用上传控件（PRD §6.2 UI-009 / UI-010）。
 *
 * - 选择文件后立即上传，进度**按真实字节数**显示（XHR upload 事件），可取消；
 * - 失败卡片保留：显示可行动文案（415/413/422 分别处理，磁盘满按
 *   `details.reason=insufficientStorage` 显示所需/可用字节）并提供「重试」（沿用同一文件
 *   重新上传）与「移除」；失败不产生半提交资产（服务端保证）；
 * - 上传中禁止重复提交（按钮禁用）；焦点留在错误卡片上可朗读；
 * - 成功不在这里提示：由调用方（绑定 document / 登记照片）决定下一步。
 */

import { useEffect, useRef, useState, type ReactNode } from "react";

import { formatBytes } from "../../lib/format";
import { describeUploadFailure, type UploadFailureInfo } from "./upload-messages";
import { uploadAsset, type AssetDto, type AssetPurpose, type UploadProgress } from "./upload";

export interface AssetUploadCardProps {
  readonly itemId: string;
  readonly purpose: AssetPurpose;
  /** `accept` 属性（如 `application/pdf`、`image/jpeg,image/png`）。 */
  readonly accept: string;
  /** 文件输入的可见标签（也是可访问名称）。 */
  readonly inputLabel: string;
  readonly hint?: ReactNode;
  readonly disabled?: boolean;
  /** 上传成功后回调（例如绑定 document 或登记照片）；抛错由调用方展示。 */
  readonly onUploaded: (asset: AssetDto) => void | Promise<void>;
  readonly testId?: string;
}

export function AssetUploadCard({
  itemId,
  purpose,
  accept,
  inputLabel,
  hint,
  disabled = false,
  onUploaded,
  testId,
}: AssetUploadCardProps) {
  const [file, setFile] = useState<File | null>(null);
  const [uploading, setUploading] = useState(false);
  const [progress, setProgress] = useState<UploadProgress | null>(null);
  const [failure, setFailure] = useState<UploadFailureInfo | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const failureRef = useRef<HTMLDivElement | null>(null);

  // 失败卡片出现时把焦点移过去（可朗读），用户可直接 Tab 到「重试」。
  useEffect(() => {
    if (failure !== null && !uploading) {
      failureRef.current?.focus();
    }
  }, [failure, uploading]);

  async function runUpload(target: File): Promise<void> {
    setUploading(true);
    setFailure(null);
    setProgress({ loaded: 0, total: target.size });
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const asset = await uploadAsset(itemId, purpose, target, target.name, {
        signal: controller.signal,
        onProgress: setProgress,
      });
      setFile(null);
      setProgress(null);
      if (inputRef.current !== null) {
        inputRef.current.value = "";
      }
      await onUploaded(asset);
    } catch (error) {
      setFailure(describeUploadFailure(error));
    } finally {
      abortRef.current = null;
      setUploading(false);
    }
  }

  function handleSelection(selected: File | null): void {
    if (selected === null) {
      return;
    }
    setFile(selected);
    void runUpload(selected);
  }

  function cancel(): void {
    abortRef.current?.abort();
  }

  function remove(): void {
    setFile(null);
    setFailure(null);
    setProgress(null);
    if (inputRef.current !== null) {
      inputRef.current.value = "";
    }
  }

  const percent =
    progress !== null && progress.total > 0
      ? Math.round((progress.loaded / progress.total) * 100)
      : null;

  return (
    <div className="upload-card" data-testid={testId}>
      <div className="upload-card__picker">
        <label className="field__label" htmlFor={`upload-input-${purpose}-${testId ?? "default"}`}>
          {inputLabel}
        </label>
        <input
          id={`upload-input-${purpose}-${testId ?? "default"}`}
          ref={inputRef}
          type="file"
          accept={accept}
          disabled={disabled || uploading}
          onChange={(event) => handleSelection(event.target.files?.[0] ?? null)}
        />
        {hint !== undefined && <p className="field__hint">{hint}</p>}
      </div>

      {uploading && progress !== null && (
        <div className="upload-card__progress">
          <p role="status" aria-live="polite" data-testid={testId !== undefined ? `${testId}-progress` : undefined}>
            正在上传 {file?.name ?? "文件"}：{formatBytes(progress.loaded)} / {formatBytes(progress.total)}
            {percent !== null && `（${percent}%）`}
          </p>
          <div
            className="progressbar"
            role="progressbar"
            aria-label="上传进度"
            aria-valuenow={progress.loaded}
            aria-valuemin={0}
            aria-valuemax={progress.total}
          >
            <span
              className="progressbar__fill"
              style={{ width: `${percent ?? 0}%` }}
            />
          </div>
          <button type="button" onClick={cancel}>
            取消上传
          </button>
        </div>
      )}

      {failure !== null && !uploading && (
        <div className="error-panel upload-card__error" role="alert" tabIndex={-1} ref={failureRef}>
          <h3>{failure.title}</h3>
          <p>{failure.message}</p>
          {failure.hint !== null && <p className="error-panel__meta">{failure.hint}</p>}
          <div className="error-panel__actions">
            <button type="button" disabled={file === null} onClick={() => file !== null && void runUpload(file)}>
              重试
            </button>
            <button type="button" onClick={remove}>
              移除
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
