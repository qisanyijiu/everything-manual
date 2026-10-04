import {
  STANDALONE_MODEL_ELEMENT_ID,
  STANDALONE_PAYLOAD_ELEMENT_ID,
  STANDALONE_PAYLOAD_VERSION,
} from "./payload";
import {
  StandaloneExportError,
  buildStandaloneHtml,
  buildStandalonePayload,
  bytesToBase64,
  serializeForScript,
  standaloneFileName,
} from "./build-html";

const MODEL = { assetId: "asset-1", revisionId: "rev-1", sha256: "a".repeat(64), validationState: "validated" };

function manifest(overrides: { hotspots?: unknown[]; model?: unknown } = {}) {
  return {
    publishedAt: "2026-10-02T08:58:15.61Z",
    knowledge: {
      model: overrides.model ?? MODEL,
      knowledge: {
        parts: [
          { id: "part-a", name: "Rear cover", description: "With screws", evidence: [{ pageNumber: 1, quote: "q" }] },
          { id: "part-b", name: "Cable</script><b>x</b>", description: "", evidence: [] },
        ],
        steps: [
          {
            id: "step-1",
            title: "Loosen",
            orderedActions: ["Loosen the screws."],
            partIds: ["part-a"],
            safetyNotes: [],
            evidence: [{ pageNumber: 1, quote: null }],
          },
        ],
        specs: [{ id: "spec-1", label: "Power", value: "DC 12 V", evidence: [] }],
      },
      hotspots: overrides.hotspots ?? [
        {
          id: "hot-ok",
          partId: "part-a",
          status: "confirmed",
          anchor: { modelRevisionId: "rev-1", modelSha256: "a".repeat(64), positionLocal: [0.1, 0.2, 0.3] },
        },
        {
          id: "hot-stale",
          partId: "part-b",
          status: "confirmed",
          anchor: { modelRevisionId: "rev-OLD", modelSha256: "b".repeat(64), positionLocal: [0, 0, 0] },
        },
      ],
    },
  };
}

const ITEM = { name: "传感器控制盒", brand: "Fixture", model: "X100" };

describe("buildStandalonePayload", () => {
  it("整理已发布知识，并只保留锚点属于发布模型版本的热点", () => {
    const payload = buildStandalonePayload(ITEM, { id: "rel-1", manifest: manifest() });
    expect(payload.schemaVersion).toBe(STANDALONE_PAYLOAD_VERSION);
    expect(payload.title).toBe("传感器控制盒");
    expect(payload.subtitle).toBe("Fixture · X100");
    expect(payload.parts.map((part) => part.id)).toEqual(["part-a", "part-b"]);
    expect(payload.steps[0]?.orderedActions).toEqual(["Loosen the screws."]);
    expect(payload.specs[0]).toMatchObject({ label: "Power", value: "DC 12 V" });
    expect(payload.hotspots).toEqual([{ id: "hot-ok", partId: "part-a", positionLocal: [0.1, 0.2, 0.3] }]);
    expect(payload.publishedAt).toBe("2026-10-02T08:58:15.61Z");
  });

  it("没有已校验模型时拒绝生成（不产出空壳 3D 页面）", () => {
    const bad = manifest({ model: { ...MODEL, validationState: "rejected" } });
    expect(() => buildStandalonePayload(ITEM, { id: "rel-1", manifest: bad })).toThrow(StandaloneExportError);
  });
});

describe("buildStandaloneHtml", () => {
  const payload = buildStandalonePayload(ITEM, { id: "rel-1", manifest: manifest() });
  const html = buildStandaloneHtml(payload, new Uint8Array([103, 108, 84, 70]), "console.log('</script>')");

  it("内嵌载荷、模型与脚本，且载荷里的 </script> 不会截断标签", () => {
    const payloadBlock = html.match(
      new RegExp(`<script id="${STANDALONE_PAYLOAD_ELEMENT_ID}" type="application/json">([\\s\\S]*?)</script>`),
    );
    expect(payloadBlock).not.toBeNull();
    const parsed = JSON.parse(payloadBlock?.[1] ?? "") as { parts: { name: string }[] };
    expect(parsed.parts[1]?.name).toBe("Cable</script><b>x</b>");
    expect(html).toContain(`<script id="${STANDALONE_MODEL_ELEMENT_ID}" type="application/octet-stream">Z2xURg==</script>`);
    expect(html).toContain("console.log('<\\/script>')");
  });

  it("用 CSP 禁止一切网络请求，标题经过 HTML 转义", () => {
    expect(html).toContain("default-src 'none'");
    expect(html).not.toMatch(/connect-src[^;"]*https?:/);
    expect(html).toContain("<title>传感器控制盒 · 交互式说明书</title>");
    const evil = buildStandaloneHtml({ ...payload, title: "<img src=x>" }, new Uint8Array(), "");
    expect(evil).toContain("<title>&lt;img src=x&gt; · 交互式说明书</title>");
  });
});

describe("helpers", () => {
  it("serializeForScript 转义 < 与行分隔符", () => {
    expect(serializeForScript({ a: "</script>\u2028\u2029" })).toBe('{"a":"\\u003c/script>\\u2028\\u2029"}');
  });

  it("bytesToBase64 处理大于单块的数据", () => {
    const bytes = new Uint8Array(0x8000 * 2 + 3).map((_, index) => index % 251);
    expect(atob(bytesToBase64(bytes)).length).toBe(bytes.length);
  });

  it("standaloneFileName 去掉路径与控制字符", () => {
    expect(standaloneFileName({ name: "a/b:c", brand: null, model: "X\u0007100" })).toBe("a_b_c-X100-3d.html");
    expect(standaloneFileName({ name: "", brand: null, model: null })).toBe("manual-3d.html");
  });
});
