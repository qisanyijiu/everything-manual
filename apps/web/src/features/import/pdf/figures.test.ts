import {
  DEFAULT_FILTER,
  connectedRegions,
  dedupeBoxes,
  dilate,
  imageBoxesFromOperators,
  plausibleFigure,
  toPixels,
  unitSquareBounds,
  type Matrix,
} from "./figures";

const OPS = { save: 10, restore: 11, transform: 12, paintImageXObject: 85, paintInlineImageXObject: 86, paintImageMaskXObject: 83 };

describe("imageBoxesFromOperators", () => {
  it("跟踪 save/restore/transform，得到位图在页面空间的包围盒", () => {
    const list = {
      fnArray: [OPS.save, OPS.transform, OPS.transform, OPS.paintImageXObject, OPS.restore, OPS.save, OPS.transform, OPS.paintImageMaskXObject, OPS.restore],
      argsArray: [null, [1, 0, 0, 1, 50, 100], [200, 0, 0, 150, 0, 0], ["img_p0_1"], null, null, [30, 0, 0, 30, 10, 10], ["mask"], null],
    };
    expect(imageBoxesFromOperators(list, OPS)).toEqual([
      { x: 50, y: 100, width: 200, height: 150 },
      { x: 10, y: 10, width: 30, height: 30 },
    ]);
  });

  it("restore 后回到外层矩阵", () => {
    const list = { fnArray: [OPS.save, OPS.transform, OPS.restore, OPS.paintImageXObject], argsArray: [null, [5, 0, 0, 5, 0, 0], null, ["x"]] };
    expect(imageBoxesFromOperators(list, OPS)).toEqual([{ x: 0, y: 0, width: 1, height: 1 }]);
  });
});

describe("几何", () => {
  it("unitSquareBounds 处理翻转矩阵", () => {
    expect(unitSquareBounds([100, 0, 0, -50, 10, 60])).toEqual({ x: 10, y: 10, width: 100, height: 50 });
  });

  it("toPixels 把 PDF 用户空间（原点左下）映射到像素空间（原点左上）", () => {
    const viewport: Matrix = [2, 0, 0, -2, 0, 1684]; // A4 高 842pt × 2
    expect(toPixels({ x: 100, y: 742, width: 200, height: 100 }, viewport)).toEqual({ x: 200, y: 0, width: 400, height: 200 });
  });

  it("dedupeBoxes 去掉被大框覆盖的小框", () => {
    const big = { x: 0, y: 0, width: 100, height: 100 };
    expect(dedupeBoxes([{ x: 10, y: 10, width: 20, height: 20 }, big, { x: 150, y: 0, width: 50, height: 50 }])).toEqual([big, { x: 150, y: 0, width: 50, height: 50 }]);
  });

  it("plausibleFigure 过滤图标、细长线条和整页背景", () => {
    const filter = { pageWidth: 1000, pageHeight: 1400, ...DEFAULT_FILTER };
    expect(plausibleFigure({ x: 0, y: 0, width: 500, height: 400 }, filter)).toBe(true);
    expect(plausibleFigure({ x: 0, y: 0, width: 40, height: 40 }, filter)).toBe(false);
    expect(plausibleFigure({ x: 0, y: 0, width: 900, height: 120 }, filter)).toBe(false);
    expect(plausibleFigure({ x: 0, y: 0, width: 1000, height: 1400 }, filter)).toBe(false);
  });
});

describe("连通区域", () => {
  it("膨胀后把分散笔画连成一块，两幅图分开", () => {
    const w = 20;
    const h = 10;
    const mask = new Uint8Array(w * h);
    for (const [x, y] of [[1, 1], [3, 1], [1, 3], [3, 3], [15, 6], [17, 7]] as const) {
      mask[y * w + x] = 1;
    }
    const regions = connectedRegions(dilate(mask, w, h, 1), w, h);
    expect(regions).toHaveLength(2);
    expect(regions[0]).toEqual({ x: 0, y: 0, width: 5, height: 5 });
  });
});
