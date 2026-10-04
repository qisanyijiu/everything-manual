// 机器狗用户全流程走查：只通过界面点击完成「登录 → 发布 → 阅读」，每一个用户必须停下来操作的界面截一张图。
// 用法（cwd = apps/web）：node em-walk.mjs <outDir>
import { chromium } from "playwright";
import fs from "node:fs";

const OUT = process.argv[2] ?? "/tmp/emwalk/shots";
const PASSWORD = process.env.EM_E2E_PASSWORD;
if (!PASSWORD) throw new Error("请通过 EM_E2E_PASSWORD 提供本次隔离实例的登录密码。");
const RESUME_JOB = process.argv[3] ?? null;
let shotStart = Number(process.argv[4] ?? 0);
fs.mkdirSync(OUT, { recursive: true });
const PDF = "/Users/wault/Downloads/CyberDog 2 产品说明书.pdf";
const WEB = "http://127.0.0.1:5173";
const browser = await chromium.launch({ channel: "chrome", headless: true, args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const p = await browser.newPage({ viewport: { width: 1440, height: 960 } });
const issues = [];
p.on("pageerror", (e) => issues.push(`pageerror: ${e.message}`));
p.on("response", (r) => { if (r.status() >= 500) issues.push(`HTTP ${r.status()} ${new URL(r.url()).pathname}`); });

let shot = shotStart;
const clicks = [];
const t0 = Date.now();
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
async function snap(name, note) {
  shot += 1;
  const file = `${String(shot).padStart(2, "0")}-${name}.png`;
  await p.screenshot({ path: `${OUT}/${file}` });
  clicks.push({ shot: file, note });
  log("shot", file, note);
}
const click = async (locator, what) => { await locator.click(); log("click", what); };

try {
  // 1 登录
  await p.goto(`${WEB}/login`);
  await p.locator('input[type="password"]').waitFor();
  await p.fill('input[type="password"]', PASSWORD);
  if (RESUME_JOB === null) await snap("login", "输入管理员密码并登录");
  await p.keyboard.press("Enter");
  await p.waitForURL((u) => !u.pathname.startsWith("/login"));

  if (RESUME_JOB === null) {
  // 2 资料库 → 新建物品
  await p.waitForTimeout(1500);
  await snap("library", "资料库首页，点「新建物品」");
  await click(p.getByRole("link", { name: "新建物品" }).first(), "新建物品");

  // 3 填写名称/型号
  await p.getByLabel("名称").waitFor();
  await p.getByLabel("名称").fill("CyberDog 2 机器狗");
  await p.getByLabel("准确型号").fill("CyberDog 2");
  await p.getByLabel("品牌").fill("小米").catch(() => {});
  await snap("item-form", "填写名称与型号，点「创建并继续」");
  await click(p.getByRole("button", { name: "创建并继续" }), "创建并继续");

  // 4 上传 PDF 并绑定
  await p.waitForURL(/import\/document/);
  await p.getByLabel("选择 PDF 文件").setInputFiles(PDF);
  await p.getByRole("button", { name: "绑定为说明书" }).waitFor({ timeout: 120000 });
  await snap("upload-pdf", "选择说明书 PDF（自动上传），点「绑定为说明书」");
  await click(p.getByRole("button", { name: "绑定为说明书" }), "绑定为说明书");
  await p.getByRole("heading", { name: "已绑定的说明书" }).waitFor({ timeout: 60000 });
  await click(p.getByRole("link", { name: /下一步/ }).first(), "下一步：视图排列");

  // 5 视图：从说明书提取 → 自动填充
  await p.getByTestId("extract-candidates").waitFor({ timeout: 60000 });
  await click(p.getByTestId("extract-candidates"), "从说明书提取候选图");
  await p.waitForFunction(() => {
    const btn = document.querySelector('[data-testid="autofill"]');
    return document.querySelectorAll('[data-testid^="arrange-card-"]').length > 0 && btn && !btn.hasAttribute("disabled");
  }, null, { timeout: 900000 });
  await click(p.getByTestId("autofill"), "按建议填入空槽");
  await p.waitForTimeout(2500);
  await snap("views", "从说明书提取产品图，按建议填入视图槽位（可拖拽修正）");
  await click(p.getByRole("link", { name: /下一步/ }).first(), "下一步：准备");

  // 6 准备：开始 → 封存
  await p.waitForTimeout(2500);
  if (await p.getByTestId("prepare-start").isVisible().catch(() => false)) await click(p.getByTestId("prepare-start"), "开始准备");
  await p.waitForFunction(() => /封存资料|准备完成 · \d+ 页/.test(document.body.innerText), null, { timeout: 900000 });
  if (await p.getByTestId("prepare-seal").isVisible().catch(() => false)) await click(p.getByTestId("prepare-seal"), "封存资料");
  await p.waitForFunction(() => /准备完成 · \d+ 页/.test(document.body.innerText), null, { timeout: 120000 });
  await snap("prepare", "浏览器逐页制作页图并封存");
  await click(p.getByRole("link", { name: /下一步/ }).first(), "下一步：预算/隐私确认");

  // 7 报价确认 → 生成
  await p.getByTestId("quote-panel").waitFor({ timeout: 180000 });
  await p.locator("#send-scope-confirm").check();
  await p.getByTestId("confirmed-at").waitFor({ timeout: 30000 });
  await p.getByTestId("generate-button").scrollIntoViewIfNeeded();
  await snap("confirm", "查看报价与将发送的资料，勾选同意后点「生成」");
  await click(p.getByTestId("generate-button"), "生成");
  await p.getByTestId("job-accepted").waitFor({ timeout: 60000 });

  }
  // 8 等待：任务中心（用户可以离开，完成后全局弹提示）
  if (RESUME_JOB === null) {
    await click(p.getByRole("link", { name: "任务中心" }).first(), "任务中心");
    await p.waitForTimeout(4000);
    await snap("jobs", "任务已受理，在任务中心看进度（可离开页面）");
  }
  if (RESUME_JOB === null) {
    // 全局通知条里的「查看生成结果」（任务列表行里也有同名链接，只认通知条）
    const done = p.locator(".notices").getByRole("link", { name: "查看生成结果" });
    await done.waitFor({ timeout: 75 * 60 * 1000 });
    await snap("done-toast", "生成完成，全局提示「查看生成结果」");
    await click(done.first(), "查看生成结果");
  } else {
    await p.goto(`${WEB}/jobs/${RESUME_JOB}`);
    await p.getByRole("link", { name: "查看生成结果（3D 预览）" }).waitFor({ timeout: 60000 });
    await p.waitForTimeout(1500);
    await snap("job-done", "任务完成：各阶段已完成，点「查看生成结果」");
    await click(p.getByRole("link", { name: "查看生成结果（3D 预览）" }), "查看生成结果");
  }

  // 9 结果页：3D + 热点 + 姿势
  await p.waitForFunction(() => /模型已加载|模型加载失败/.test(document.body.innerText), null, { timeout: 180000 });
  await p.waitForTimeout(4000);
  const sit = p.getByTestId("interaction-panel").getByRole("button", { name: "坐下", exact: true });
  if (await sit.count()) { await click(sit, "姿势：坐下"); await p.waitForTimeout(1800); }
  await snap("result", "生成结果：3D 模型、自动热点、姿势/动作可直接点");
  await click(p.getByRole("link", { name: "去复核并发布" }), "去复核并发布");

  // 10 复核：批量确认热点 + 批量确认文字 + 模型声明
  await p.getByTestId("publish-panel").waitFor({ timeout: 120000 });
  await p.waitForFunction(() => /模型已加载|已在浏览器成功打开此模型/.test(document.body.innerText), null, { timeout: 120000 });
  await p.waitForTimeout(5000);
  await snap("review", "复核工作区：待确认的知识、候选热点、发布检查清单");
  if (await p.getByTestId("confirm-candidates").count()) {
    await click(p.getByTestId("confirm-candidates"), "确认全部候选热点");
    await p.waitForFunction(() => !document.querySelector('[data-testid="confirm-candidates"]'), null, { timeout: 30000 });
  }
  const all = p.getByTestId("confirm-all-entities");
  if (await all.count()) {
    await click(all, "确认全部文字事实");
    await p.waitForFunction(() => !document.querySelector('[data-testid="confirm-all-entities"]'), null, { timeout: 30000 });
  }
  const loaded = p.locator("#review-modelLoaded");
  await p.waitForFunction(() => { const b = document.querySelector("#review-modelLoaded"); return b && !b.disabled; }, null, { timeout: 60000 }).catch(() => {});
  if (await loaded.isEnabled()) await click(loaded, "已在浏览器成功打开此模型");
  const confirmed = p.locator("#review-modelConfirmed");
  await p.waitForFunction(() => { const b = document.querySelector("#review-modelConfirmed"); return b && !b.disabled; }, null, { timeout: 30000 }).catch(() => {});
  if (await confirmed.isEnabled()) await click(confirmed, "我已核对模型与资料一致");
  await p.waitForTimeout(2500);
  const checklist = await p.getByTestId("publish-panel").innerText();
  await p.getByTestId("publish-panel").scrollIntoViewIfNeeded();
  await snap("publish-ready", "批量确认热点与文字、声明已核对模型后，发布清单状态");

  // 11 发布
  const publish = p.getByTestId("publish-panel").getByRole("button", { name: /发布（生成不可变版本）|重新检查并发布/ });
  const canPublish = await publish.isEnabled().catch(() => false);
  let published = false;
  if (canPublish) {
    await click(publish, "发布");
    await p.getByTestId("publish-success").waitFor({ timeout: 60000 }).then(() => { published = true; }).catch(() => {});
  }
  if (!published) issues.push(`未能发布：${checklist.replace(/\s+/g, " ").slice(0, 400)}`);
  else {
    await snap("published", "已发布不可变版本");
    const link = p.getByTestId("publish-success").getByRole("link", { name: "打开阅读器" });
    if (await link.count()) {
      await click(link, "打开发布版本");
      await p.waitForFunction(() => /模型已加载|模型加载失败/.test(document.body.innerText), null, { timeout: 180000 }).catch(() => {});
      await p.waitForTimeout(4000);
      const stand = p.getByTestId("interaction-panel").getByRole("button", { name: "握手", exact: true });
      if (await stand.count()) { await click(stand, "姿势：握手"); await p.waitForTimeout(1800); }
      await snap("reader", "最终用户看到的发布版：3D 说明书 + 热点 + 姿势");
    }
  }
} catch (e) {
  issues.push(`FAILED: ${String(e?.message ?? e).slice(0, 400)}`);
  await p.screenshot({ path: `${OUT}/zz-fail.png` }).catch(() => {});
} finally {
  const summary = { shots: shot, clicks, issues, minutes: Math.round((Date.now() - t0) / 6000) / 10 };
  fs.writeFileSync(`${OUT}/walk.json`, JSON.stringify(summary, null, 2));
  log("DONE", JSON.stringify({ shots: shot, issues }));
  await browser.close();
}
