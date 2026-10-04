// 参数化浏览器端到端：登录 → 建物品 → 上传 PDF → 视图候选 → 资料准备 → 报价确认 → 生成 → 结果页交互。
// 用法：node e2e.mjs <tag> <pdf> <name> <model>   （cwd 必须是 apps/web，以便解析 playwright）
// 产出：/tmp/em5/<tag>/{result.json, *.png, run.log}
import { chromium } from "playwright";
import fs from "node:fs";

const [tag, PDF, NAME, MODEL] = process.argv.slice(2);
const OUT = `/tmp/em5/${tag}`;
fs.mkdirSync(OUT, { recursive: true });
const WEB = "http://127.0.0.1:5173";
const started = Date.now();
const logLines = [];
const log = (...a) => {
  const line = `${new Date().toISOString().slice(11, 19)} [${tag}] ${a.join(" ")}`;
  console.log(line);
  logLines.push(line);
  fs.writeFileSync(`${OUT}/run.log`, logLines.join("\n"));
};
const result = { tag, name: NAME, model: MODEL, pdf: PDF, interventions: [], bugs: [], pageErrors: [] };
const save = () => fs.writeFileSync(`${OUT}/result.json`, JSON.stringify(result, null, 2));

const browser = await chromium.launch({ channel: "chrome", headless: true, args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const p = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
p.on("pageerror", (e) => result.pageErrors.push(e.message));
p.on("response", (r) => { if (r.status() >= 500) result.bugs.push(`HTTP ${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`); });

const api = (m, path, body, headers = {}) => p.evaluate(async ([m, path, body, headers]) => {
  const s = await (await fetch("/api/v1/auth/session")).json();
  const r = await fetch("/api/v1" + path, { method: m, headers: { "content-type": "application/json", "x-csrf-token": s.data.csrfToken, ...headers }, body: body ? JSON.stringify(body) : undefined });
  const text = await r.text();
  return { status: r.status, etag: r.headers.get("etag"), json: text ? JSON.parse(text) : null };
}, [m, path, body, headers]);

async function step(label, fn) {
  const t0 = Date.now();
  try {
    const v = await fn();
    log(`${label} ok (${Math.round((Date.now() - t0) / 1000)}s)`);
    return v;
  } catch (e) {
    await p.screenshot({ path: `${OUT}/fail-${label.replace(/\W+/g, "_")}.png` }).catch(() => {});
    result.failedAt = label;
    result.error = String(e?.message ?? e).slice(0, 600);
    log(`${label} FAILED: ${result.error}`);
    throw e;
  }
}

try {
  await step("login", async () => {
    await p.goto(`${WEB}/login`);
    await p.fill('input[type="password"]', "KRLBEQX9DsiV", { timeout: 60000 });
    await p.keyboard.press("Enter");
    await p.waitForURL((u) => !u.pathname.startsWith("/login"));
  });

  const item = await step("create-item", async () => (await api("POST", "/items", { name: NAME, model: MODEL })).json.data.id);
  result.itemId = item;

  await step("upload-pdf", async () => {
    const b64 = fs.readFileSync(PDF).toString("base64");
    const asset = await p.evaluate(async ([item, b64]) => {
      const s = await (await fetch("/api/v1/auth/session")).json();
      const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const fd = new FormData();
      fd.append("purpose", "document");
      fd.append("file", new Blob([bin], { type: "application/pdf" }), "manual.pdf");
      const r = await fetch(`/api/v1/items/${item}/assets`, { method: "POST", headers: { "x-csrf-token": s.data.csrfToken }, body: fd });
      return { status: r.status, body: await r.json() };
    }, [item, b64]);
    if (asset.status >= 300) throw new Error(`asset upload ${asset.status} ${JSON.stringify(asset.body).slice(0, 300)}`);
    const doc = await api("POST", `/items/${item}/documents`, { sourceAssetId: asset.body.data.id, title: `${NAME} 说明书` });
    if (doc.status >= 300) throw new Error(`document ${doc.status} ${JSON.stringify(doc.json).slice(0, 300)}`);
  });

  await step("views", async () => {
    await p.goto(`${WEB}/items/${item}/import/views`);
    await p.getByTestId("extract-candidates").waitFor({ timeout: 60000 });
    await p.getByTestId("extract-candidates").click();
    await p.waitForFunction(() => {
      const cards = document.querySelectorAll('[data-testid^="arrange-card-"]').length;
      const btn = document.querySelector('[data-testid="autofill"]');
      const t = document.body.innerText;
      return (cards > 0 && btn && !btn.hasAttribute("disabled")) || /未找到|没有找到|0 张候选/.test(t);
    }, null, { timeout: 900000 });
    result.viewCandidates = await p.locator('[data-testid^="arrange-card-"]').count();
    if (result.viewCandidates === 0) throw new Error("PDF 中没有提取到视图候选");
    await p.getByTestId("autofill").click();
    await p.waitForTimeout(1500);
    result.slots = await p.evaluate(() => ["front", "left", "back", "right", "detail"].filter((v) => document.querySelector(`[data-testid="arrange-slot-${v}"] img`)));
    await p.waitForFunction(() => !document.querySelector('[data-testid="arrangement-unsaved"]'), null, { timeout: 15000 }).catch(async () => {
      result.interventions.push("视图排布未自动保存，需手动点「保存」");
      if (await p.getByTestId("save-arrangement").isEnabled()) { await p.getByTestId("save-arrangement").click(); await p.waitForTimeout(2000); }
    });
    result.savedViews = (await api("GET", `/items/${item}/photos`)).json.data.map((x) => x.view);
    await p.screenshot({ path: `${OUT}/1-views.png` });
  });

  await step("prepare", async () => {
    await p.goto(`${WEB}/items/${item}/import/prepare`);
    await p.waitForTimeout(3000);
    if (await p.getByTestId("prepare-start").isVisible()) await p.getByTestId("prepare-start").click();
    await p.waitForFunction(() => { const t = document.body.innerText; return t.includes("封存资料") || t.includes("已就绪") || /准备完成 · \d+ 页/.test(t); }, null, { timeout: 900000 });
    if (await p.getByTestId("prepare-seal").isVisible()) await p.getByTestId("prepare-seal").click();
    await p.waitForFunction(() => /已就绪|准备完成 · \d+ 页/.test(document.body.innerText), null, { timeout: 60000 });
  });

  const jobId = await step("confirm-generate", async () => {
    await p.goto(`${WEB}/items/${item}/import/confirm`);
    await p.getByTestId("quote-panel").waitFor({ timeout: 180000 });
    await p.locator("#send-scope-confirm").check();
    await p.getByTestId("confirmed-at").waitFor({ timeout: 30000 });
    await p.getByTestId("generate-button").click();
    await p.getByTestId("job-accepted").waitFor({ timeout: 60000 });
    await p.screenshot({ path: `${OUT}/2-accepted.png` });
    return (await p.getByTestId("job-accepted").locator("code").first().innerText()).trim();
  });
  result.jobId = jobId;
  save();

  await step("generation", async () => {
    const deadline = Date.now() + 75 * 60 * 1000;
    let lastSig = "";
    while (Date.now() < deadline) {
      const r = await api("GET", `/jobs/${jobId}`);
      const d = r.json.data;
      const sig = d.status + " " + d.stages.filter((s) => s.status !== "succeeded").map((s) => `${s.stageKind}#${s.batchIndex}:${s.status}`).join(",");
      if (sig !== lastSig) { log("job", sig.slice(0, 300)); lastSig = sig; }
      if (["succeeded", "failed", "cancelled"].includes(d.status)) {
        result.jobStatus = d.status;
        result.stages = d.stages.map((s) => ({ kind: s.stageKind, batch: s.batchIndex, status: s.status, usage: s.usage, error: s.lastError?.slice(0, 200) }));
        if (d.status !== "succeeded") throw new Error(`job ${d.status}`);
        result.draftId = d.draftId;
        return;
      }
      // 结果未知：模拟用户在任务详情页点「授权替代提交」（记录为人工介入）
      for (const s of d.stages.filter((s) => s.status === "submission_unknown")) {
        result.interventions.push(`对账 ${s.stageKind}#${s.batchIndex}：${(s.lastError ?? "").slice(0, 80)}`);
        const fresh = await api("GET", `/jobs/${jobId}`);
        const res = Object.fromEntries(fresh.json.data.reservations.map((x) => [x.provider, x.reservedMinor]));
        const rr = await api("POST", `/jobs/${jobId}/reconcile`, {
          action: "authorizeReplacement", stageId: s.id, acknowledgeDuplicateRisk: true,
          limits: { tripoCreditMinor: res.tripo ?? 0, manualAiUsdMicros: res.manual_ai ?? 0 },
        }, { "If-Match": fresh.etag });
        log("reconcile", s.stageKind, s.batchIndex, rr.status, rr.status >= 300 ? JSON.stringify(rr.json).slice(0, 300) : rr.json.data.notice.slice(0, 120));
      }
      // 剩余可重试的 needs_input / failed：模拟用户点「重试」
      const after = (await api("GET", `/jobs/${jobId}`));
      if (!after.json.data.stages.some((s) => s.status === "submission_unknown")) {
        for (const s of after.json.data.stages.filter((s) => (s.status === "needs_input" || s.status === "failed") && s.retry?.allowed)) {
          const cur = await api("GET", `/jobs/${jobId}`);
          result.interventions.push(`重试 ${s.stageKind}#${s.batchIndex}：${(s.needsInput?.[0]?.message ?? s.lastError ?? "").slice(0, 80)}`);
          const rr = await api("POST", `/jobs/${jobId}/retry`, { stageId: s.id }, { "If-Match": cur.etag, "Idempotency-Key": crypto.randomUUID() });
          log("retry", s.stageKind, s.batchIndex, rr.status);
        }
      }
      await p.waitForTimeout(10000);
    }
    throw new Error("generation timed out");
  });

  await step("result-page", async () => {
    await p.goto(`${WEB}/jobs/${jobId}/result`);
    await p.waitForFunction(() => /模型已加载|模型加载失败/.test(document.body.innerText), null, { timeout: 180000 });
    await p.waitForTimeout(4000);
    const draft = (await api("GET", `/items/${item}/drafts/${result.draftId}`)).json.data;
    const k = draft.knowledge;
    const inter = k.interactive ?? {};
    result.draft = {
      parts: k.knowledge?.parts?.length ?? 0,
      steps: k.knowledge?.steps?.length ?? 0,
      hotspots: k.hotspots?.length ?? 0,
      partNodes: inter.partsModel?.nodeNames?.length ?? 0,
      bindings: inter.bindings?.length ?? 0,
      actions: (inter.actions ?? []).map((a) => a.label),
      poses: (inter.poses ?? []).map((x) => x.label),
    };
    result.modelLoaded = await p.evaluate(() => document.body.innerText.includes("模型已加载"));
    const panel = p.getByTestId("interaction-panel");
    result.panelButtons = (await panel.count()) ? await panel.getByRole("button").allInnerTexts() : [];
    await p.screenshot({ path: `${OUT}/3-result.png` });
    const clickable = result.panelButtons.filter((b) => b !== "复原");
    if (clickable.length) {
      await panel.getByRole("button", { name: clickable[0], exact: true }).first().click();
      await p.waitForTimeout(1800);
      await p.screenshot({ path: `${OUT}/4-interaction.png` });
      result.clicked = clickable[0];
      result.pressedAfterClick = await panel.locator('[aria-pressed="true"]').allInnerTexts();
    }
    // 点击一个热点（三维视图中的热点按钮）
    const hs = p.locator('[data-testid^="hotspot-"]');
    result.hotspotButtons = await hs.count();
    if (!result.modelLoaded) result.bugs.push("结果页模型未加载");
    if (result.draft.partNodes > 0 && result.draft.bindings === 0) result.bugs.push("有分件但没有任何绑定");
  });
  result.ok = true;
} catch {
  result.ok = false;
} finally {
  result.minutes = Math.round((Date.now() - started) / 6000) / 10;
  save();
  log("DONE ok=" + result.ok, "minutes=" + result.minutes);
  await browser.close();
}
