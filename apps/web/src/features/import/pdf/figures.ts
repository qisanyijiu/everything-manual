/**
 * 从说明书 PDF 页中拆出"可能是产品视图"的图片区域（ADR-044；视图候选）。
 *
 * 两类来源（同一页可同时出现）：
 * - **embedded**：页面内嵌位图。遍历 operator list，跟踪 save/restore/transform，记录每次绘制
 *   位图时的单位方块在页面空间的包围盒；随后从高分辨率渲染图裁剪该区域（这样遮罩、色彩空间、
 *   叠加线条都与页面所见一致，不需要自己解码 XObject）；
 * - **region**：矢量线稿（很多相机说明书的外观图就是矢量）。以"不含文字"的方式渲染页面，在降采样
 *   的墨迹掩膜上找大的连通区域，过滤掉细长的线条/表格框后裁剪。
 *
 * 本文件的几何与区域查找是纯函数（可在 Vitest 里验证）；与 pdf.js / canvas 的交互在 `extract.ts`。
 */

export interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** 2D 仿射矩阵 `[a, b, c, d, e, f]`（与 PDF / canvas 约定一致）。 */
export type Matrix = readonly [number, number, number, number, number, number];

export const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];

export function multiply(m: Matrix, n: Matrix): Matrix {
  return [
    m[0] * n[0] + m[2] * n[1],
    m[1] * n[0] + m[3] * n[1],
    m[0] * n[2] + m[2] * n[3],
    m[1] * n[2] + m[3] * n[3],
    m[0] * n[4] + m[2] * n[5] + m[4],
    m[1] * n[4] + m[3] * n[5] + m[5],
  ];
}

/** 单位方块 (0,0)-(1,1) 经矩阵变换后的轴对齐包围盒（PDF 绘制位图即绘制单位方块）。 */
export function unitSquareBounds(m: Matrix): Box {
  const xs = [m[4], m[0] + m[4], m[2] + m[4], m[0] + m[2] + m[4]];
  const ys = [m[5], m[1] + m[5], m[3] + m[5], m[1] + m[3] + m[5]];
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

/** operator list 的最小形态（pdf.js `PDFOperatorList`）。 */
export interface OperatorListLike {
  readonly fnArray: ArrayLike<number>;
  readonly argsArray: ArrayLike<unknown>;
}

export interface OpCodes {
  readonly save: number;
  readonly restore: number;
  readonly transform: number;
  readonly paintImageXObject: number;
  readonly paintInlineImageXObject: number;
  readonly paintImageMaskXObject: number;
}

/**
 * 遍历 operator list，返回每次绘制位图时的包围盒（PDF 用户空间，原点左下、y 向上）。
 * 只考虑 CTM（pdf.js 已把表单 XObject 展开为 `paintFormXObjectBegin` + transform）。
 */
export function imageBoxesFromOperators(list: OperatorListLike, ops: OpCodes): Box[] {
  const stack: Matrix[] = [];
  let ctm: Matrix = IDENTITY;
  const boxes: Box[] = [];
  for (let i = 0; i < list.fnArray.length; i += 1) {
    const fn = list.fnArray[i];
    if (fn === ops.save) {
      stack.push(ctm);
    } else if (fn === ops.restore) {
      ctm = stack.pop() ?? IDENTITY;
    } else if (fn === ops.transform) {
      const args = list.argsArray[i] as readonly number[] | undefined;
      if (Array.isArray(args) && args.length >= 6) {
        ctm = multiply(ctm, args.slice(0, 6) as unknown as Matrix);
      }
    } else if (fn === ops.paintImageXObject || fn === ops.paintInlineImageXObject || fn === ops.paintImageMaskXObject) {
      boxes.push(unitSquareBounds(ctm));
    }
  }
  return boxes;
}

/** PDF 用户空间（原点左下）→ 渲染像素空间（原点左上），`viewportTransform` 来自 pdf.js viewport。 */
export function toPixels(box: Box, viewportTransform: Matrix): Box {
  const corners: [number, number][] = [
    [box.x, box.y],
    [box.x + box.width, box.y],
    [box.x, box.y + box.height],
    [box.x + box.width, box.y + box.height],
  ];
  const t = viewportTransform;
  const px = corners.map(([x, y]) => [t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5]] as const);
  const xs = px.map((p) => p[0]);
  const ys = px.map((p) => p[1]);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

export function area(box: Box): number {
  return Math.max(0, box.width) * Math.max(0, box.height);
}

export function intersection(a: Box, b: Box): number {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const w = Math.min(a.x + a.width, b.x + b.width) - x;
  const h = Math.min(a.y + a.height, b.y + b.height) - y;
  return w > 0 && h > 0 ? w * h : 0;
}

/** 去掉被更大框大部分覆盖的框（同一张图常被重复绘制或拆成多块）。 */
export function dedupeBoxes(boxes: readonly Box[], overlap = 0.8): Box[] {
  const sorted = [...boxes].sort((a, b) => area(b) - area(a));
  const kept: Box[] = [];
  for (const box of sorted) {
    if (!kept.some((k) => intersection(k, box) >= overlap * area(box))) {
      kept.push(box);
    }
  }
  return kept;
}

export interface FigureFilter {
  readonly pageWidth: number;
  readonly pageHeight: number;
  /** 占页面面积的最小比例（太小 = 图标/装饰）。 */
  readonly minAreaRatio: number;
  /** 最小边长像素。 */
  readonly minSide: number;
  /** 宽高比上限（细长 = 线条、分隔条、表格行）。 */
  readonly maxAspect: number;
  /** 占页面面积的最大比例（整页背景图不是视图）。 */
  readonly maxAreaRatio: number;
}

export const DEFAULT_FILTER = { minAreaRatio: 0.02, minSide: 96, maxAspect: 4, maxAreaRatio: 0.92 } as const;

export function plausibleFigure(box: Box, filter: FigureFilter): boolean {
  const ratio = area(box) / (filter.pageWidth * filter.pageHeight);
  const aspect = Math.max(box.width, box.height) / Math.max(1, Math.min(box.width, box.height));
  return (
    ratio >= filter.minAreaRatio &&
    ratio <= filter.maxAreaRatio &&
    Math.min(box.width, box.height) >= filter.minSide &&
    aspect <= filter.maxAspect
  );
}

/**
 * 在二值墨迹掩膜上找连通区域（4 邻接），返回包围盒（掩膜坐标）。
 * `mask[y * width + x] = 1` 表示该格有墨迹；调用方先降采样并膨胀，使同一幅线稿连成一片。
 */
export function connectedRegions(mask: Uint8Array, width: number, height: number): Box[] {
  const seen = new Uint8Array(mask.length);
  const boxes: Box[] = [];
  const queue = new Int32Array(mask.length);
  for (let start = 0; start < mask.length; start += 1) {
    if (mask[start] === 0 || seen[start] === 1) {
      continue;
    }
    let head = 0;
    let tail = 0;
    queue[tail++] = start;
    seen[start] = 1;
    let minX = width;
    let minY = height;
    let maxX = 0;
    let maxY = 0;
    while (head < tail) {
      const index = queue[head++] as number;
      const x = index % width;
      const y = (index - x) / width;
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
      const neighbours = [x > 0 ? index - 1 : -1, x < width - 1 ? index + 1 : -1, y > 0 ? index - width : -1, y < height - 1 ? index + width : -1];
      for (const n of neighbours) {
        if (n >= 0 && mask[n] === 1 && seen[n] === 0) {
          seen[n] = 1;
          queue[tail++] = n;
        }
      }
    }
    boxes.push({ x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1 });
  }
  return boxes;
}

/** 二值掩膜膨胀（方形结构元素，半径 r）：让同一幅线稿的分散笔画连通。 */
export function dilate(mask: Uint8Array, width: number, height: number, radius: number): Uint8Array {
  const out = new Uint8Array(mask.length);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      if (mask[y * width + x] === 0) {
        continue;
      }
      for (let dy = -radius; dy <= radius; dy += 1) {
        const yy = y + dy;
        if (yy < 0 || yy >= height) {
          continue;
        }
        for (let dx = -radius; dx <= radius; dx += 1) {
          const xx = x + dx;
          if (xx >= 0 && xx < width) {
            out[yy * width + xx] = 1;
          }
        }
      }
    }
  }
  return out;
}

/** 给框加边距并裁到页面内。 */
export function pad(box: Box, margin: number, maxWidth: number, maxHeight: number): Box {
  const x = Math.max(0, box.x - margin);
  const y = Math.max(0, box.y - margin);
  return {
    x,
    y,
    width: Math.min(maxWidth, box.x + box.width + margin) - x,
    height: Math.min(maxHeight, box.y + box.height + margin) - y,
  };
}
