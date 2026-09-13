/**
 * RD 布局守卫：资料库行几何（BUG-005 修复的几何证据；PRD §6.1 断点 / §6.2 UI-005）。
 *
 * 背景：T16 在行内 `.item-row__actions` 里新增了常驻禁用原因（`.item-row__note`）后，
 * 行内四列 grid 的末端 `auto` 轨道按说明文字 max-content 定宽，把身份列挤到 0px
 * （1280/1366px 下名称逐字换行、单行高 358px，QA 回合 18 BUG-005）。修复后本文件固化：
 * - 桌面 1024/1280/1366/1440/1600/1920px：名称列宽 ≥ 120px、行高 ≤ 140px，
 *   说明文字独占整行（不参与列宽竞争）；
 * - 窄屏 767/375px（<768px）：行仍为单列堆叠、名称可读、摘要抽屉可开可关（不回退）。
 *
 * 阈值与 QA 的 `qa-t16-independent.spec.ts` QA-10 一致（名称 ≥120px）；本文件是 RD 侧守卫，
 * 几何写入 `artifacts/web-mvp/t16-rd-fix/`（不覆盖 QA 的 `t16-qa/`）。
 */

import fs from "node:fs";
import path from "node:path";

import { expect, test } from "@playwright/test";

import { captureTo, loginViaUi, runtime, seedItem } from "./helpers";

const password = (): string => runtime().password;
const apiBase = (): string => runtime().apiBase;

/** 名称列可读阈值（与 QA-10 相同）。 */
const MIN_NAME_WIDTH = 120;
/** 行高上限：两行身份 + 一行说明 + 内边距与间距的合理上界（逐字换行时曾达 358px）。 */
const MAX_ROW_HEIGHT = 140;

const WIDE_WIDTHS = [1024, 1280, 1366, 1440, 1600, 1920] as const;
const NARROW_WIDTHS = [767, 375] as const;

interface RowGeometry {
  readonly gridTemplateColumns: string;
  readonly row: { x: number; y: number; width: number; height: number };
  readonly parts: Record<string, { x: number; y: number; width: number; height: number } | null>;
}

/** 一次 evaluate 原子取整行几何（避免视口切换/重渲染期间的 stale 句柄）。 */
async function measureRow(row: import("@playwright/test").Locator): Promise<RowGeometry> {
  return row.evaluate((node) => {
    const rect = (el: Element) => {
      const r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, width: r.width, height: r.height };
    };
    const pick = (selector: string) => {
      const el = node.querySelector(selector);
      return el === null ? null : rect(el);
    };
    return {
      gridTemplateColumns: getComputedStyle(node).gridTemplateColumns,
      row: rect(node),
      parts: {
        identity: pick(".item-row__identity"),
        name: pick(".item-row__name"),
        status: pick(".item-row__status"),
        time: pick(".item-row__time"),
        actions: pick(".item-row__actions"),
        note: pick(".item-row__note"),
      },
    };
  });
}

async function resizeTo(
  page: import("@playwright/test").Page,
  row: import("@playwright/test").Locator,
  width: number,
): Promise<RowGeometry> {
  await page.setViewportSize({ width, height: 900 });
  await expect(row).toBeVisible();
  // 断点切换会让 PageLayout 重挂载（wide/mid/narrow 三套 DOM），等一帧再量。
  await page.waitForTimeout(100);
  return measureRow(row);
}

test("RD-ROW-1 桌面 1024–1920px：身份列可读、行高正常、说明独占整行（BUG-005 守卫）", async ({
  page,
  request,
}) => {
  await seedItem(request, apiBase(), password(), "RD 行布局守卫");
  await loginViaUi(page, "", password());
  await page.goto("/");

  const row = page.locator(".item-row").filter({ hasText: "RD 行布局守卫" });
  await expect(row).toBeVisible();

  const measurements: Record<string, RowGeometry> = {};
  for (const width of WIDE_WIDTHS) {
    const geometry = await resizeTo(page, row, width);
    measurements[String(width)] = geometry;
    const { row: rowBox, parts } = geometry;
    const name = parts.name;
    const identity = parts.identity;
    const note = parts.note;

    // 身份列与名称必须获得真实宽度（BUG-005 修复前：身份列 0px、名称 22.8px）。
    expect(
      name?.width ?? 0,
      `${width}px：名称列宽 ${name?.width}px（阈值 ≥${MIN_NAME_WIDTH}）`,
    ).toBeGreaterThanOrEqual(MIN_NAME_WIDTH);
    expect(identity?.width ?? 0, `${width}px：身份列宽 ${identity?.width}px`).toBeGreaterThanOrEqual(
      MIN_NAME_WIDTH,
    );
    // 行高正常（修复前 1280px 达 357.7px）。
    expect(
      rowBox.height,
      `${width}px：行高 ${rowBox.height}px（阈值 ≤${MAX_ROW_HEIGHT}）`,
    ).toBeLessThanOrEqual(MAX_ROW_HEIGHT);
    // 说明文字独占整行：宽度≈行内容宽，位于操作行之下（不参与四列竞争）。
    expect(note?.width ?? 0, `${width}px：说明行宽 ${note?.width}px`).toBeGreaterThanOrEqual(
      rowBox.width - 32,
    );
    expect(note?.y ?? 0, `${width}px：说明行未落到操作行之下`).toBeGreaterThan(rowBox.y + 20);
    // 四列轨道都在（未退化为逐字换行的单字列）。
    expect(geometry.gridTemplateColumns.trim().split(/\s+/).length, `${width}px 轨道数`).toBe(4);
  }

  const outDir = path.join(
    path.resolve(import.meta.dirname, "../../../../artifacts/web-mvp/t16-rd-fix"),
  );
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(
    path.join(outDir, "library-row-geometry.json"),
    JSON.stringify(measurements, null, 2),
  );
  await resizeTo(page, row, 1280);
  await captureTo("t16-rd-fix", page, "01-library-row-1280");
  await resizeTo(page, row, 1366);
  await captureTo("t16-rd-fix", page, "02-library-row-1366");
});

test("RD-ROW-2 窄屏 <768px：行单列堆叠、名称可读、摘要抽屉可开可关（不回退）", async ({
  page,
  request,
}) => {
  await seedItem(request, apiBase(), password(), "RD 窄屏行守卫");
  await loginViaUi(page, "", password());
  await page.goto("/");
  const row = page.locator(".item-row").filter({ hasText: "RD 窄屏行守卫" });
  await expect(row).toBeVisible();

  const measurements: Record<string, RowGeometry> = {};
  for (const width of NARROW_WIDTHS) {
    const geometry = await resizeTo(page, row, width);
    measurements[String(width)] = geometry;
    // 单列：计算值只有一个轨道（PRD §6.1.1 窄屏单栏；行由四列退化为堆叠）。
    expect(
      geometry.gridTemplateColumns.trim().split(/\s+/).length,
      `${width}px：行仍是多列（${geometry.gridTemplateColumns}）`,
    ).toBe(1);
    const name = geometry.parts.name;
    expect(name?.width ?? 0, `${width}px：名称列宽 ${name?.width}px`).toBeGreaterThanOrEqual(200);
    const note = geometry.parts.note;
    expect(note?.width ?? 0, `${width}px：说明行宽 ${note?.width}px`).toBeGreaterThanOrEqual(200);
  }
  const outDir = path.join(
    path.resolve(import.meta.dirname, "../../../../artifacts/web-mvp/t16-rd-fix"),
  );
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(
    path.join(outDir, "library-row-geometry-narrow.json"),
    JSON.stringify(measurements, null, 2),
  );

  // 抽屉（narrow <768px 单栏 + 抽屉，PRD §6.1.1）仍可开可关。
  await resizeTo(page, row, 375);
  const trigger = page.getByRole("button", { name: "资料库摘要" });
  await expect(trigger).toBeVisible();
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "资料库摘要" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await captureTo("t16-rd-fix", page, "03-library-row-375");
});
