/**
 * 向导第 2 步：说明书原件（PRD §6.1.2 `/items/:itemId/import/document`；
 * §6.2 UI-009（上传）、UI-011（绑定 document）、UI-016（拒绝说明）、UI-019（向导导航））。
 *
 * - 上传 PDF（`POST /items/{id}/assets` purpose=document，真实字节进度、可取消、可重试）；
 * - 绑定为 document（`POST /items/{id}/documents`）：标题可选、出处链接可选，
 *   **服务端不访问 sourceUrl**（固定提示）；不做物品级来源链接字段（A-16/D-1）；
 * - 已绑定的说明书来自服务端（刷新/返回上一步不丢）；
 * - 加密 / >100 页 PDF 的权威拒绝在准备页（ADR-003），这里只做说明与提示。
 */

import { useState } from "react";
import { Link, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { createDocument } from "../../api/endpoints";
import { EmptyNote } from "../../components/EmptyState";
import { FormErrorSummary, readFieldErrors, TextField, type FieldError } from "../../components/form";
import { Skeleton } from "../../components/Skeleton";
import { useNotify } from "../../components/notifications";
import { formatLocalDateTime } from "../../lib/format";
import { useQueryClient } from "@tanstack/react-query";
import { AssetUploadCard } from "./AssetUploadCard";
import { WizardNav, WizardSteps } from "./WizardSteps";
import { JobSnapshotNotice } from "../library/JobSnapshotNotice";
import { useItemDetail, useItemDocuments, itemKeys } from "../library/items";
import type { AssetDto } from "./upload";

const PDF_LIMITS_HINT =
  "原件 PDF ≤50 MiB、≤100 页；加密 PDF 首版不支持（准备阶段会明确拒绝并说明原因）。";

export function DocumentStepPage() {
  const { itemId } = useParams();
  const id = itemId ?? "";
  const itemQuery = useItemDetail(id === "" ? null : id);
  const documentsQuery = useItemDocuments(id === "" ? null : id);

  const documents = documentsQuery.data?.documents ?? [];

  return (
    <section className="page document-step" aria-labelledby="document-step-title">
      <WizardSteps currentSegment="import/document" itemId={id} />
      <h1 id="document-step-title">说明书原件</h1>
      <p className="page__lead">
        {itemQuery.data?.data.name ?? "物品"}：上传 PDF 原件并绑定为说明书；
        逐页准备（页文字与页图）在第 4 步完成。
      </p>

      {/* UI-021：更换说明书不影响已开始任务的冻结快照。 */}
      <JobSnapshotNotice itemId={id} />

      {(itemQuery.isPending || documentsQuery.isPending) && (
        <Skeleton label="正在读取说明书清单…" rows={3} />
      )}

      {itemQuery.error !== null && itemQuery.error !== undefined && (
        <div className="error-panel" role="alert">
          <h2>无法读取物品</h2>
          <p>{describeError(itemQuery.error).message}</p>
          <button type="button" onClick={() => void itemQuery.refetch()}>
            重试
          </button>
        </div>
      )}

      {documentsQuery.isError && (
        <div className="error-panel" role="alert">
          <h2>说明书清单加载失败</h2>
          <p>{describeError(documentsQuery.error).message}</p>
          <button type="button" onClick={() => void documentsQuery.refetch()}>
            重试
          </button>
        </div>
      )}

      {documentsQuery.data !== undefined && documents.length === 0 && (
        <EmptyNote>还没有绑定说明书原件：先在下方上传 PDF。</EmptyNote>
      )}

      {documents.length > 0 && (
        <section className="panel" aria-labelledby="bound-documents-title">
          <h2 id="bound-documents-title">已绑定的说明书</h2>
          <ul className="entity-list">
            {documents.map((document) => (
              <li key={document.id}>
                <span className="entity-list__title">{document.title}</span>
                <span className="entity-list__meta">
                  原件 sha256 {document.sourceSha256.slice(0, 12)}… · 绑定于{" "}
                  {formatLocalDateTime(document.createdAt)}
                </span>
                {document.sourceUrl !== null && document.sourceUrl !== undefined && (
                  <span className="entity-list__meta">
                    出处链接：{document.sourceUrl}（仅记录，服务器不访问）
                  </span>
                )}
              </li>
            ))}
          </ul>
          <p>
            <Link to={`/items/${id}/import/prepare`}>去准备（第 4 步）</Link>
          </p>
        </section>
      )}

      <section className="panel" aria-labelledby="upload-document-title">
        <h2 id="upload-document-title">上传 PDF 原件</h2>
        <Binder itemId={id} />
        <p className="field__hint">{PDF_LIMITS_HINT}</p>
      </section>

      <WizardNav
        currentSegment="import/document"
        itemId={id}
        nextDisabled={documents.length === 0}
        nextDisabledReason="先绑定一份说明书原件，才能继续到视图排列与准备。"
      />
    </section>
  );
}

/** 上传 + 绑定：上传得到资产后填写标题/出处并 `POST /items/{id}/documents`。 */
function Binder({ itemId }: { itemId: string }) {
  const queryClient = useQueryClient();
  const notify = useNotify();
  const [asset, setAsset] = useState<AssetDto | null>(null);
  const [title, setTitle] = useState("");
  const [sourceUrl, setSourceUrl] = useState("");
  const [binding, setBinding] = useState(false);
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [formError, setFormError] = useState<string | null>(null);

  async function bind(): Promise<void> {
    if (asset === null) {
      return;
    }
    setBinding(true);
    setFieldErrors([]);
    setFormError(null);
    try {
      const created = await createDocument(itemId, {
        sourceAssetId: asset.id,
        title: title.trim() === "" ? (asset.originalName ?? "说明书原件") : title.trim(),
        sourceUrl: sourceUrl.trim() === "" ? null : sourceUrl.trim(),
      });
      await queryClient.invalidateQueries({ queryKey: itemKeys.documents(itemId) });
      setAsset(null);
      setTitle("");
      setSourceUrl("");
      notify(`已绑定说明书「${created.data.title}」`);
    } catch (error) {
      setFieldErrors(isApiError(error) ? readFieldErrors(error.details) : []);
      setFormError(describeError(error).message);
    } finally {
      setBinding(false);
    }
  }

  return (
    <div className="binder">
      <AssetUploadCard
        itemId={itemId}
        purpose="document"
        accept="application/pdf,.pdf"
        inputLabel="选择 PDF 文件"
        hint="上传完成后需要再确认「绑定为说明书」才会写入资料记录。"
        onUploaded={(uploaded) => {
          setAsset(uploaded);
          setTitle(uploaded.originalName ?? "");
          setFieldErrors([]);
          setFormError(null);
        }}
        testId="document-upload"
      />

      {asset !== null && (
        <form
          className="binder__form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            if (!binding) {
              void bind();
            }
          }}
        >
          <p className="binder__file" role="status">
            待绑定文件：{asset.originalName ?? "(未命名)"}（{asset.mime}，
            sha256 {asset.sha256.slice(0, 12)}…）
          </p>
          <FormErrorSummary id="document-bind-errors" errors={fieldErrors} />
          {formError !== null && fieldErrors.length === 0 && (
            <div className="form-errors" role="alert">
              <p>{formError}</p>
            </div>
          )}
          <TextField
            field="title"
            label="标题（可选）"
            value={title}
            onChange={setTitle}
            hint="默认取原文件名；仅用于展示。"
          />
          <div className="field">
            <label className="field__label" htmlFor="field-sourceUrl">
              出处链接（可选）
            </label>
            <input
              id="field-sourceUrl"
              className="field__input"
              type="url"
              value={sourceUrl}
              aria-describedby="field-sourceUrl-hint"
              onChange={(event) => setSourceUrl(event.target.value)}
            />
            <p className="field__hint" id="field-sourceUrl-hint">
              来源链接仅作出处记录，服务器不会访问该地址。
            </p>
          </div>
          <div className="binder__actions">
            <button type="submit" className="button-primary" disabled={binding}>
              {binding ? "绑定中…" : "绑定为说明书"}
            </button>
            <button
              type="button"
              onClick={() => {
                setAsset(null);
                setFieldErrors([]);
                setFormError(null);
              }}
              disabled={binding}
            >
              取消绑定
            </button>
          </div>
          <p className="field__hint">
            取消绑定不会删除已上传的资产（本版本不提供删除资产的入口）。
          </p>
        </form>
      )}
    </div>
  );
}
