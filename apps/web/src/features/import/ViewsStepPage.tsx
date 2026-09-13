/**
 * 向导第 3 步：视图排列（PRD §6.1.2 `/items/:itemId/import/views`；
 * §6.2 UI-009（上传）、UI-010（失败重试）、UI-012（槽位与占用）、UI-013（缺项）、UI-019（导航））。
 *
 * - 五个槽位：front（必需）/ left / back / right / detail；同一视图最多一张照片；
 * - 槽位为空 → 选择文件上传（purpose=photo，JPEG/PNG ≤20 MiB）；
 *   槽位已有照片 → 缩略图 + 「替换照片」（PATCH assetId）+ 槽位改选（PATCH view）；
 * - 同视图占用（后端 422 `details.reason=viewOccupied`）给出可行动文案：
 *   本版本**不提供删除**，用「替换照片」更新该视图，或把另一张改到别的槽位；
 * - detail 槽位注明「特写仅用于理解与核对，不发送给 Tripo」；
 * - 完成度缺项（缺 front / 缺侧面）常驻显示，并链接到可修复的步骤。
 */

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { assetContentUrl, createPhoto, patchPhoto, type PhotoDto } from "../../api/endpoints";
import { Skeleton } from "../../components/Skeleton";
import { readReason } from "../../components/form";
import { AssetUploadCard } from "./AssetUploadCard";
import { JobSnapshotNotice } from "../library/JobSnapshotNotice";
import { MissingItemsList } from "./MissingItemsList";
import { WizardNav, WizardSteps } from "./WizardSteps";
import { VIEW_LABELS, VIEW_ORDER, generationGaps, photosByView, type ViewSlot } from "./views";
import { itemKeys, useItemDetail, useItemPhotos } from "../library/items";
import type { AssetDto } from "./upload";

const PHOTO_HINT = "JPEG / PNG，单文件 ≤20 MiB；HEIC/WebP 请先自行转换。";

export function ViewsStepPage() {
  const { itemId } = useParams();
  const id = itemId ?? "";
  const itemQuery = useItemDetail(id === "" ? null : id);
  const photosQuery = useItemPhotos(id === "" ? null : id);
  const queryClient = useQueryClient();
  const [photoError, setPhotoError] = useState<string | null>(null);
  const [busySlot, setBusySlot] = useState<ViewSlot | null>(null);

  const photos = photosQuery.data?.photos ?? [];
  const slots = photosByView(photos);

  async function refreshPhotos(): Promise<void> {
    await queryClient.invalidateQueries({ queryKey: itemKeys.photos(id) });
  }

  /** 同视图占用与其它照片写入错误的统一处理（可行动文案，不用通用错误）。 */
  function describePhotoFailure(error: unknown, view: ViewSlot): string {
    const info = describeError(error);
    const reason = isApiError(error) ? readReason(error.details) : null;
    if (reason === "viewOccupied") {
      return `「${VIEW_LABELS[view]}（${view}）」已有照片：请先移除或改选。当前版本不提供删除照片的入口，可用该槽位的「替换照片」更新内容，或把另一张照片改到其它槽位。`;
    }
    return info.message;
  }

  async function registerPhoto(view: ViewSlot, asset: AssetDto): Promise<void> {
    setBusySlot(view);
    setPhotoError(null);
    try {
      await createPhoto(id, { assetId: asset.id, view });
      await refreshPhotos();
    } catch (error) {
      setPhotoError(describePhotoFailure(error, view));
    } finally {
      setBusySlot(null);
    }
  }

  async function replacePhoto(photo: PhotoDto, view: ViewSlot, asset: AssetDto): Promise<void> {
    setBusySlot(view);
    setPhotoError(null);
    try {
      await patchPhoto(id, photo.id, { assetId: asset.id }, `"r${photo.revision}"`);
      await refreshPhotos();
    } catch (error) {
      setPhotoError(describePhotoFailure(error, view));
    } finally {
      setBusySlot(null);
    }
  }

  async function changeView(photo: PhotoDto, target: ViewSlot): Promise<void> {
    setBusySlot(target);
    setPhotoError(null);
    try {
      await patchPhoto(id, photo.id, { view: target }, `"r${photo.revision}"`);
      await refreshPhotos();
    } catch (error) {
      setPhotoError(describePhotoFailure(error, target));
    } finally {
      setBusySlot(null);
    }
  }

  const gaps = generationGaps({
    itemId: id,
    // 本步不读取 preparation：准备相关的缺项在第 4/5 步呈现，这里只提示视图缺项。
    preparationState: "ready",
    photos,
    generationCapability: null,
  }).filter((gap) => gap.code === "missingFrontView" || gap.code === "missingSideView");

  return (
    <section className="page views-step" aria-labelledby="views-step-title">
      <WizardSteps currentSegment="import/views" itemId={id} />
      <h1 id="views-step-title">视图排列</h1>
      <p className="page__lead">
        {itemQuery.data?.data.name ?? "物品"}
        ：为照片分配 front/left/back/right/detail 槽位；方向以物品自身为参照（不是观察者视角）。
      </p>

      {/* UI-021：替换照片/改槽位不影响已开始任务的冻结快照。 */}
      <JobSnapshotNotice itemId={id} />

      {(itemQuery.isPending || photosQuery.isPending) && (
        <Skeleton label="正在读取视图槽位…" rows={4} />
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

      {photosQuery.isError && (
        <div className="error-panel" role="alert">
          <h2>视图槽位加载失败</h2>
          <p>{describeError(photosQuery.error).message}</p>
          <button type="button" onClick={() => void photosQuery.refetch()}>
            重试
          </button>
        </div>
      )}

      {photoError !== null && (
        <div className="error-panel" role="alert" data-testid="photo-error">
          <h2>照片登记失败</h2>
          <p>{photoError}</p>
          <button type="button" onClick={() => setPhotoError(null)}>
            知道了
          </button>
        </div>
      )}

      {photosQuery.data !== undefined && (
        <ul className="view-slots">
          {VIEW_ORDER.map((view) => {
            const photo = slots.get(view);
            const label = VIEW_LABELS[view];
            return (
              <li className="view-slot" key={view} data-testid={`view-slot-${view}`}>
                <h2 className="view-slot__title">
                  {label}（{view}）
                  {view === "front" && <span className="view-slot__required">必需</span>}
                </h2>
                {view === "detail" && (
                  <p className="view-slot__note">特写仅用于理解与核对，不发送给 Tripo。</p>
                )}

                {photo === undefined ? (
                  <AssetUploadCard
                    itemId={id}
                    purpose="photo"
                    accept="image/jpeg,image/png"
                    inputLabel={`上传${label}视图照片`}
                    hint={PHOTO_HINT}
                    disabled={busySlot !== null}
                    onUploaded={(asset) => registerPhoto(view, asset)}
                    testId={`photo-upload-${view}`}
                  />
                ) : (
                  <div className="view-slot__photo">
                    <img
                      className="view-slot__thumb"
                      src={assetContentUrl(photo.assetId)}
                      alt={`${label}视图照片`}
                    />
                    <div className="view-slot__tools">
                      <AssetUploadCard
                        itemId={id}
                        purpose="photo"
                        accept="image/jpeg,image/png"
                        inputLabel={`替换${label}视图照片`}
                        disabled={busySlot !== null}
                        onUploaded={(asset) => replacePhoto(photo, view, asset)}
                        testId={`photo-replace-${view}`}
                      />
                      <div className="field">
                        <label className="field__label" htmlFor={`photo-view-${view}`}>
                          {label}槽位改选视图
                        </label>
                        <select
                          id={`photo-view-${view}`}
                          className="field__input"
                          value={photo.view}
                          disabled={busySlot !== null}
                          onChange={(event) => {
                            const target = event.target.value as ViewSlot;
                            if (target !== photo.view) {
                              void changeView(photo, target);
                            }
                          }}
                        >
                          {VIEW_ORDER.map((option) => (
                            <option key={option} value={option}>
                              {VIEW_LABELS[option]}（{option}）
                            </option>
                          ))}
                        </select>
                      </div>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <MissingItemsList
        title="生成前还缺"
        gaps={gaps}
        emptyText="视图已满足多视图生成的最小集合（front + 侧面之一）。"
      />

      <WizardNav currentSegment="import/views" itemId={id} />
    </section>
  );
}
