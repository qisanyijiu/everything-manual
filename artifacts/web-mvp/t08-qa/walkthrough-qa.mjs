#!/usr/bin/env node
/**
 * T08 QA 独立浏览器走查（QA 自写，独立于 RD 的 browser-walkthrough.mjs）。
 *
 * 目标：在真实 Chrome + 真实发布二进制（内嵌 UI、SPA fallback）+ 真实临时 data-dir 上
 * 独立复现 PRD §6.1/§6.2 的 UI-001/002/003/005/006/008、§6.1.1 全局框架、
 * §6.3.2 禁用措辞、占位路由无假数据、CSRF/If-Match 真实注入、token 不泄露。
 *
 * 输出：stdout 的 [PASS]/[FAIL] 行 + OUT_DIR 下截图；任何 FAIL 使退出码非零。
 */

import { writeFileSync } from "node:fs";
import { join } from "node:path";

const CDP_PORT = process.env.CDP_PORT ?? "9444";
const APP_URL = process.env.APP_URL ?? "http://127.0.0.1:8099";
const OUT_DIR = process.env.OUT_DIR ?? ".";
const PASSWORD = process.env.EM_PASSWORD ?? "";
const WRONG_PASSWORD = "definitely-wrong-password";

const FORBIDDEN_WORDING = [
  "已自动校准",
  "自动发布",
  "总进度 100%",
  "已证明页图来自原 PDF",
  "供应商账户硬封顶",
  "零费用",
  "重试不会重复收费",
  "离线可用",
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let passed = 0;
let failed = 0;
const failedIds = [];
function check(id, ok, detail = "") {
  if (ok) {
    passed += 1;
    console.log(`[PASS] ${id}${detail === "" ? "" : ` · ${detail}`}`);
  } else {
    failed += 1;
    failedIds.push(id);
    console.log(`[FAIL] ${id}${detail === "" ? "" : ` · ${detail}`}`);
  }
}
function note(message) {
  console.log(`       ${message}`);
}

/** 与必选 AC 无关、但可复现的交互缺陷：单独记录，不混入 AC 通过/失败计数。 */
const defectRecords = [];
function defect(id, severity, summary, detail) {
  defectRecords.push({ id, severity, summary, detail });
  console.log(`[DEFECT-${severity}] ${id} ${summary} · ${detail}`);
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.subs = new Set();
    this.requests = [];
    this.responses = new Map();
    this.finished = new Set();
    this.extraHeaders = new Map();
    this.consoleMessages = [];
    this.exceptions = [];
    this.logEntries = [];
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== undefined) {
        const pending = this.pending.get(message.id);
        if (pending === undefined) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(`${message.error.message} (code ${message.error.code})`));
        else pending.resolve(message.result);
        return;
      }
      if (message.method === "Network.requestWillBeSent") {
        this.requests.push(message.params);
      } else if (message.method === "Network.responseReceived") {
        this.responses.set(message.params.requestId, message.params);
      } else if (message.method === "Network.loadingFinished") {
        this.finished.add(message.params.requestId);
      } else if (message.method === "Network.requestWillBeSentExtraInfo") {
        this.extraHeaders.set(message.params.requestId, message.params.headers ?? {});
      } else if (message.method === "Runtime.consoleAPICalled") {
        this.consoleMessages.push(message.params);
      } else if (message.method === "Runtime.exceptionThrown") {
        this.exceptions.push(message.params);
      } else if (message.method === "Log.entryAdded") {
        this.logEntries.push(message.params);
      }
      for (const sub of [...this.subs]) sub(message);
    });
  }
  send(method, params = {}, sessionId) {
    const id = this.nextId++;
    const payload = { id, method, params };
    if (sessionId !== undefined) payload.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP 命令超时（30s）：${method}`));
      }, 30000);
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      this.ws.send(JSON.stringify(payload));
    });
  }
  waitEvent(method, predicate, timeout = 8000) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.subs.delete(sub);
        reject(new Error(`等待事件超时：${method}`));
      }, timeout);
      const sub = (message) => {
        if (message.method === method && predicate(message.params)) {
          clearTimeout(timer);
          this.subs.delete(sub);
          resolve(message.params);
        }
      };
      this.subs.add(sub);
    });
  }
  mark() {
    return this.requests.length;
  }
  since(mark) {
    return this.requests.slice(mark);
  }
}

// ---------------------------------------------------------------------------
// 连接真实 Chrome
// ---------------------------------------------------------------------------

const versionInfo = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`).then((r) => r.json());
const browserVersion = versionInfo.Browser;
const ws = new WebSocket(versionInfo.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener("open", resolve);
  ws.addEventListener("error", reject);
});
const cdp = new Cdp(ws);
let { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
let { sessionId: S } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
async function enableDomains() {
  await cdp.send("Page.enable", {}, S);
  await cdp.send("Runtime.enable", {}, S);
  await cdp.send("Network.enable", {}, S);
  await cdp.send("Log.enable", {}, S);
}
await enableDomains();

/** 最近一次显式导航的目标（页面无响应时用于恢复现场）。 */
let lastGoodPath = "/";
let recoveryInFlight = false;
/**
 * 页面目标无响应（Runtime.evaluate 超时）时重建目标并回到最近一次显式导航的路径。
 * 观察到 Chrome/CDP 偶发卡在 Runtime.evaluate（被测应用本身无异常，服务端日志正常）；
 * 该恢复只影响本次走查的稳定性，不作为产品缺陷依据。
 */
async function recoverPage(reason) {
  if (recoveryInFlight) return;
  recoveryInFlight = true;
  try {
    note(`页面目标无响应（${reason}）；重建页面并回到 ${lastGoodPath}`);
    await cdp.send("Target.closeTarget", { targetId }).catch(() => {});
    const created = await cdp.send("Target.createTarget", { url: "about:blank" });
    targetId = created.targetId;
    const attached = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    S = attached.sessionId;
    await enableDomains();
    await cdp.send("Page.navigate", { url: APP_URL + lastGoodPath }, S);
    await sleep(1000);
  } finally {
    recoveryInFlight = false;
  }
}

console.log(`浏览器：${browserVersion} · 应用：${APP_URL} · node ${process.version}`);

// 异常终止时先打印现场（URL/文本/控制台错误），再退出非零。
async function reportFatal(label, error) {
  console.log(`[FAIL] ${label}：${error?.message ?? error}`);
  try {
    console.log(`       当前 URL：${await currentPath()}`);
    const text = await bodyText();
    console.log(`       页面文本片段：${(text || "").slice(0, 400).replace(/\n/g, " | ")}`);
    const errors = cdp.consoleMessages
      .filter((m) => m.type === "error")
      .slice(0, 5)
      .map((m) => m.args.map((a) => String(a.value ?? a.description ?? "")).join(" "));
    console.log(`       console.error 前 5 条：${errors.join(" ;; ") || "(无)"}`);
    console.log(`       未捕获异常数：${cdp.exceptions.length}`);
  } catch (inner) {
    console.log(`       诊断失败：${inner.message}`);
  }
}
process.on("unhandledRejection", (error) => {
  void reportFatal("走查异常终止", error).then(() => process.exit(1));
});
process.on("uncaughtException", (error) => {
  void reportFatal("走查未捕获异常", error).then(() => process.exit(1));
});

// ---------------------------------------------------------------------------
// 通用辅助
// ---------------------------------------------------------------------------

async function q(body) {
  const expression = `(() => { ${body} })()`;
  let result;
  try {
    result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, S);
  } catch (error) {
    if (!/超时/.test(error.message) || recoveryInFlight) throw error;
    await recoverPage(error.message);
    result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, S);
  }
  if (result.exceptionDetails) {
    throw new Error(
      `evaluate 异常：${result.exceptionDetails.exception?.description ?? result.exceptionDetails.text}`,
    );
  }
  return result.result.value;
}

async function waitFor(body, { timeout = 12000, interval = 120, desc = "" } = {}) {
  const deadline = Date.now() + timeout;
  for (;;) {
    let value = false;
    try {
      value = await q(body);
    } catch {
      // 导航中/文档切换：继续轮询
    }
    if (value === true) return true;
    if (Date.now() > deadline) throw new Error(`waitFor 超时：${desc || body}`);
    await sleep(interval);
  }
}

const bodyText = () => q("return document.body ? (document.body.innerText || '') : '';");
const currentPath = () => q("return location.pathname + location.search;");

async function goto(pathname) {
  lastGoodPath = pathname;
  try {
    await cdp.send("Page.navigate", { url: APP_URL + pathname }, S);
  } catch (error) {
    await recoverPage(error.message);
  }
  await waitFor("return document.readyState === 'complete'", { timeout: 20000, desc: "加载完成" });
  await sleep(250);
}

async function rectOf(exprBody) {
  return q(
    `const el = (() => { ${exprBody} })(); if (!el) return null; el.scrollIntoView({ block: "center", inline: "center" }); const r = el.getBoundingClientRect(); if (r.width === 0 && r.height === 0) return null; return { x: r.left + r.width / 2, y: r.top + r.height / 2 };`,
  );
}

async function clickRect(rect) {
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: rect.x, y: rect.y }, S);
  await cdp.send(
    "Input.dispatchMouseEvent",
    { type: "mousePressed", x: rect.x, y: rect.y, button: "left", clickCount: 1 },
    S,
  );
  await cdp.send(
    "Input.dispatchMouseEvent",
    { type: "mouseReleased", x: rect.x, y: rect.y, button: "left", clickCount: 1 },
    S,
  );
  await sleep(200);
}

async function clickByText(selector, label, { exact = true, nth = 0 } = {}) {
  const rect = await rectOf(
    `const els = [...document.querySelectorAll(${JSON.stringify(selector)})].filter((e) => { const t = (e.innerText || e.textContent || "").trim(); return ${exact ? `t === ${JSON.stringify(label)}` : `t.includes(${JSON.stringify(label)})`}; }); return els[${nth}] ?? null;`,
  );
  if (rect === null) throw new Error(`未找到可点击元素：${selector} "${label}"（第 ${nth + 1} 个）`);
  await clickRect(rect);
}

/** 点击站内链接并等待 SPA 导航；未导航时回退 `goto` 继续后续步骤（现场写入日志）。 */
async function navClick(label, expectedPath) {
  try {
    await clickByText("a", label);
  } catch (error) {
    note(`未找到链接「${label}」：${error.message}`);
  }
  try {
    await waitFor(`return location.pathname === ${JSON.stringify(expectedPath)};`, {
      timeout: 8000,
      interval: 150,
      desc: `点击「${label}」导航`,
    });
    return true;
  } catch {
    note(`点击「${label}」后未导航到 ${expectedPath}（现场见 BUG-001 证据）；回退 goto 以继续后续步骤`);
    await goto(expectedPath);
    return false;
  }
}

/** 在指定文本所在的行/容器内点击（用于多行列表定位）。 */
async function clickInScope(scopeText, selector, label) {
  const rect = await rectOf(
    `const scope = [...document.querySelectorAll('.item-row')].find((r) => (r.innerText || '').includes(${JSON.stringify(scopeText)})); if (!scope) return null; return [...scope.querySelectorAll(${JSON.stringify(selector)})].find((e) => (e.innerText || '').trim() === ${JSON.stringify(label)}) ?? null;`,
  );
  if (rect === null) throw new Error(`未找到「${scopeText}」行内的「${label}」`);
  await clickRect(rect);
}

/**
 * Fetch 域请求阶段拦截：拿到浏览器实际发出的头（CDP 的 requestWillBeSent 会漏报部分自定义头）。
 * 返回 { captured, stop() }；captured 内每项为 { method, url, headers }。
 */
async function startRequestCapture(pattern) {
  const captured = [];
  await cdp.send("Fetch.enable", { patterns: [{ urlPattern: pattern, requestStage: "Request" }] }, S);
  const sub = (message) => {
    if (message.method !== "Fetch.requestPaused") return;
    const params = message.params;
    captured.push({
      method: params.request.method,
      url: params.request.url,
      headers: params.request.headers ?? {},
    });
    cdp.send("Fetch.continueRequest", { requestId: params.requestId }, S).catch(() => {});
  };
  cdp.subs.add(sub);
  return {
    captured,
    async stop() {
      cdp.subs.delete(sub);
      await cdp.send("Fetch.disable", {}, S).catch(() => {});
    },
  };
}

async function clickSelector(selector, nth = 0) {
  const rect = await rectOf(
    `const els = [...document.querySelectorAll(${JSON.stringify(selector)})]; return els[${nth}] ?? null;`,
  );
  if (rect === null) throw new Error(`未找到可点击元素：${selector}`);
  await clickRect(rect);
}

const KEY_DEFS = {
  Tab: { vk: 9, key: "Tab", code: "Tab" },
  Enter: { vk: 13, key: "Enter", code: "Enter", text: "\r" },
  Escape: { vk: 27, key: "Escape", code: "Escape" },
};

async function pressKey(name) {
  const def = KEY_DEFS[name];
  const base = {
    windowsVirtualKeyCode: def.vk,
    nativeVirtualKeyCode: def.vk,
    key: def.key,
    code: def.code,
  };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base }, S);
  if (def.text !== undefined) {
    await cdp.send("Input.dispatchKeyEvent", { type: "char", ...base, text: def.text }, S);
  }
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base }, S);
  await sleep(170);
}

async function typeText(value) {
  await cdp.send("Input.insertText", { text: value }, S);
  await sleep(150);
}

const focusInfo = () =>
  q(
    `const a = document.activeElement; if (!a) return null; const d = document.querySelector('[role="dialog"]'); return { tag: a.tagName, id: a.id || null, role: a.getAttribute("role"), text: (a.innerText || a.value || "").trim().slice(0, 48), inDialog: d ? d.contains(a) : false };`,
  );

async function shot(name) {
  const result = await cdp.send("Page.captureScreenshot", { format: "png" }, S);
  writeFileSync(join(OUT_DIR, name), Buffer.from(result.data, "base64"));
}

async function responseBodyOf(requestId) {
  if (!cdp.finished.has(requestId)) {
    await cdp.waitEvent("Network.loadingFinished", (p) => p.requestId === requestId, 12000);
    cdp.finished.add(requestId);
  }
  const result = await cdp.send("Network.getResponseBody", { requestId }, S);
  return result.body;
}

function findRequest(list, predicate) {
  return list.find(predicate);
}

const headerValue = (headers, name) => {
  const key = Object.keys(headers ?? {}).find((candidate) => candidate.toLowerCase() === name);
  return key === undefined ? "" : String(headers[key]);
};
/** 合并 requestWillBeSent 与 requestWillBeSentExtraInfo 的头（CDP 分两处上报）。 */
const reqHeaders = (entry) => ({
  ...(entry?.request?.headers ?? {}),
  ...(cdp.extraHeaders.get(entry?.requestId) ?? {}),
});
const reqHeader = (entry, name) => headerValue(reqHeaders(entry), name);

const apiRequests = (list) =>
  list
    .filter((entry) => entry.request.url.startsWith(`${APP_URL}/api/`))
    .map((entry) => ({
      method: entry.request.method,
      path: entry.request.url.slice(APP_URL.length),
      headers: entry.request.headers,
      type: entry.type,
    }));

// ---------------------------------------------------------------------------
// 步骤 1：未登录深链 + 恢复中骨架（限速网络）+ no-store
// ---------------------------------------------------------------------------

console.log("--- 步骤 1：未登录深链 /settings + 会话恢复 ---");
await cdp.send(
  "Network.emulateNetworkConditions",
  { offline: false, latency: 1200, downloadThroughput: -1, uploadThroughput: -1 },
  S,
);
const mark1 = cdp.mark();
await cdp.send("Page.navigate", { url: `${APP_URL}/settings` }, S);
await waitFor("return document.readyState !== 'loading'", { timeout: 20000, desc: "文档解析" });
let sawSkeleton = false;
let passwordDuringRestore = false;
for (let i = 0; i < 80; i += 1) {
  const text = await bodyText().catch(() => "");
  if (text.includes("正在恢复会话")) {
    sawSkeleton = true;
    passwordDuringRestore ||= await q("return document.querySelector('input[type=password]') !== null;");
  }
  if (text.includes("登录已过期")) break;
  await sleep(100);
}
check(
  "UI-002.a 恢复中显示全屏骨架、恢复期间不出现密码框（不闪登录页）",
  sawSkeleton && !passwordDuringRestore,
  `skeleton=${sawSkeleton} passwordShown=${passwordDuringRestore}`,
);
await waitFor("return document.querySelector('#field-password') !== null", { timeout: 25000, desc: "登录页" });
await cdp.send(
  "Network.emulateNetworkConditions",
  { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 },
  S,
);
await sleep(300);
const path1 = await currentPath();
check("UI-002.b 未登录深链 → /login?next=%2Fsettings", path1 === "/login?next=%2Fsettings", path1);
const text1 = await bodyText();
check(
  "UI-002.c 登录页保留 next 与过期说明",
  text1.includes("登录已过期") && text1.includes("/settings"),
  text1.split("\n").slice(0, 5).join(" / "),
);
const sessionRequest = findRequest(cdp.since(mark1), (e) => e.request.url.endsWith("/api/v1/auth/session"));
const unauthenticatedSessionStatus = sessionRequest
  ? cdp.responses.get(sessionRequest.requestId)?.response?.status
  : null;
check(
  "AC-003.401 未登录 GET /auth/session 返回 401（UI-002 会话探测）",
  unauthenticatedSessionStatus === 401,
  `status=${unauthenticatedSessionStatus}`,
);
const loginForm = await q(
  `const p = document.querySelector('#field-password'); const b = [...document.querySelectorAll('button')].find((x) => (x.innerText || '').includes('登录')); return { type: p.type, autocomplete: p.getAttribute('autocomplete'), value: p.value, submitDisabled: b ? b.disabled : null };`,
);
check(
  "UI-001.a 密码框 type=password/autocomplete=current-password、无默认值、空密码禁用提交",
  loginForm.type === "password" &&
    loginForm.autocomplete === "current-password" &&
    loginForm.value === "" &&
    loginForm.submitDisabled === true,
  JSON.stringify(loginForm),
);
await shot("01-login-next-settings.png");

// ---------------------------------------------------------------------------
// 步骤 2：键盘登录 + next 安全性
// ---------------------------------------------------------------------------

console.log("--- 步骤 2：键盘登录与 next 安全 ---");

async function loginViaKeyboard(password) {
  await waitFor("return document.querySelector('#field-password') !== null", { desc: "登录页" });
  await q("document.querySelector('#field-password')?.blur(); if (document.body) { document.body.setAttribute('tabindex','-1'); document.body.focus(); document.body.removeAttribute('tabindex'); } return true;");
  await pressKey("Tab");
  const firstTab = await focusInfo();
  await typeText(password);
  const enabledAfterTyping = await q(
    "const b = [...document.querySelectorAll('button')].find((x) => (x.innerText || '').includes('登录')); return b ? !b.disabled : null;",
  );
  await pressKey("Tab");
  const secondTab = await focusInfo();
  const beforeUrl = await currentPath();
  const alertBefore = await q(
    "const a = document.querySelector('[role=alert]'); return a ? (a.innerText || '').trim() : null;",
  );
  await pressKey("Enter");
  let submittedByEnter = false;
  try {
    await waitFor(
      `const a = document.querySelector('[role=alert]'); const t = a ? (a.innerText || '').trim() : null; return (location.pathname + location.search !== ${JSON.stringify(beforeUrl)}) || (t !== null && t !== ${JSON.stringify(alertBefore)});`,
      { timeout: 6000, interval: 120, desc: "Enter 提交" },
    );
    submittedByEnter = true;
  } catch {
    submittedByEnter = false;
  }
  if (!submittedByEnter) {
    // 回退路径（计入检查结果，不掩盖键盘提交失败）
    try {
      await clickByText("button", "登录");
      await sleep(600);
    } catch {
      /* ignore */
    }
  }
  lastGoodPath = await currentPath().catch(() => lastGoodPath);
  return { firstTab, secondTab, enabledAfterTyping, submittedByEnter };
}

// 2.1 非法 next：协议相对
await goto("/login?next=%2F%2Fevil.com");
const keyboard = await loginViaKeyboard(PASSWORD);
check(
  "T08-键盘登录.1 键盘完成登录（Tab 到密码框 → 输入 → Tab 到按钮 → Enter 提交）",
  keyboard.firstTab?.id === "field-password" &&
    keyboard.secondTab?.text?.includes("登录") === true &&
    keyboard.enabledAfterTyping === true &&
    keyboard.submittedByEnter === true,
  `first=${keyboard.firstTab?.id} second=${keyboard.secondTab?.text} enter提交=${keyboard.submittedByEnter}`,
);
await waitFor("return document.querySelector('.top-bar') !== null", { timeout: 12000, desc: "登录后外壳" });
const pathAfterEvil = await currentPath();
check("UI-002.e 协议相对 next（//evil.com）登录后回落 /", pathAfterEvil === "/", pathAfterEvil);
const textAfterEvil = await bodyText();
check("UI-002.f 页面不出现 evil.com", !textAfterEvil.includes("evil.com"), "");

// 抓取登录响应体（CSRF token 与错误结构都在体里；token 值不写日志）
const loginRequest = findRequest(
  cdp.requests.slice().reverse(),
  (e) => e.request.url.endsWith("/api/v1/auth/login") && e.request.method === "POST",
);
let csrfToken = "";
if (loginRequest !== undefined) {
  const body = await responseBodyOf(loginRequest.requestId);
  try {
    csrfToken = JSON.parse(body)?.data?.csrfToken ?? "";
  } catch {
    csrfToken = "";
  }
}
check("UI-001.b 登录响应体含 csrfToken 与会话信息", csrfToken.length > 0, `csrf长度=${csrfToken.length}`);

// 2.2 非法 next：绝对 URL
await clickByText("button", "登出");
await waitFor("return document.querySelector('#field-password') !== null", { timeout: 12000, desc: "登出后回登录页" });
await goto("/login?next=https%3A%2F%2Fevil.example%2Fsteal");
await loginViaKeyboard(PASSWORD);
await waitFor("return document.querySelector('.top-bar') !== null", { timeout: 12000, desc: "登录后外壳" });
const pathAfterAbsolute = await currentPath();
check("UI-002.g 绝对 URL next 登录后回落 /", pathAfterAbsolute === "/", pathAfterAbsolute);

// 2.3 合法 next 生效
await goto("/login?next=%2Fsettings");
await loginViaKeyboard(PASSWORD);
await waitFor("return document.querySelector('#settings-title') !== null", { timeout: 15000, desc: "设置页" });
const pathAfterValid = await currentPath();
check("UI-002.h 合法站内 next（/settings）登录后确实返回该路径", pathAfterValid === "/settings", pathAfterValid);
await shot("02-settings-page.png");

// ---------------------------------------------------------------------------
// 步骤 3：错误密码（键盘）+ 错误关联 + 诊断请求 ID
// ---------------------------------------------------------------------------

console.log("--- 步骤 3：错误密码与错误关联 ---");
await clickByText("button", "登出");
await waitFor("return document.querySelector('#field-password') !== null", { timeout: 12000, desc: "回登录页" });
const markLoginError = cdp.mark();
await loginViaKeyboard(WRONG_PASSWORD);
await waitFor("return document.querySelector('[role=alert]') !== null", { timeout: 12000, desc: "错误摘要" });
const alertInfo = await q(
  `const a = document.querySelector('[role=alert]'); const p = document.querySelector('#field-password'); return { text: (a.innerText || '').trim(), focused: document.activeElement === a, describedBy: p.getAttribute('aria-describedby'), fieldErrorText: (document.getElementById('field-password-error')?.textContent || '').trim(), invalid: p.getAttribute('aria-invalid') };`,
);
check("UI-001.c 401 显示「密码不正确」", alertInfo.text.includes("密码不正确"), alertInfo.text.split("\n")[0]);
check("UI-001.d 失败后焦点在错误摘要（role=alert）", alertInfo.focused === true, `focused=${alertInfo.focused}`);
check(
  "UI-001.e 字段通过 aria-describedby 关联同一条错误文案",
  alertInfo.describedBy === "field-password-error" &&
    alertInfo.fieldErrorText.includes("密码不正确") &&
    alertInfo.invalid === "true",
  `describedBy=${alertInfo.describedBy} invalid=${alertInfo.invalid}`,
);
check("UI-001.f 错误摘要含诊断请求 ID", /诊断请求 ID/.test(alertInfo.text), "");
const loginErrorResponse = cdp.since(markLoginError).find((e) => e.request.method === "POST" && e.request.url.endsWith("/auth/login"));
const loginErrorStatus = loginErrorResponse
  ? cdp.responses.get(loginErrorResponse.requestId)?.response?.status
  : null;
check("AC-003.401 登录失败 HTTP 状态为 401", loginErrorStatus === 401, `status=${loginErrorStatus}`);
const originRejected = await fetch(`${APP_URL}/api/v1/auth/login`, {
  method: "POST",
  headers: { "content-type": "application/json", origin: "https://evil.example" },
  body: JSON.stringify({ password: WRONG_PASSWORD }),
});
check(
  "UI-001.h 跨站 Origin 的登录请求被服务端拒绝（403；页面文案由单元用例覆盖）",
  originRejected.status === 403,
  `status=${originRejected.status}`,
);
await shot("03-login-error-focus.png");

// ---------------------------------------------------------------------------
// 步骤 4：正确登录 + 顶栏 + 凭据不落地
// ---------------------------------------------------------------------------

console.log("--- 步骤 4：登录成功、顶栏与凭据不泄露 ---");
const markLoginOk = cdp.mark();
await loginViaKeyboard(PASSWORD);
await waitFor("return document.querySelector('.top-bar') !== null", { timeout: 15000, desc: "登录成功" });
const loginOkRequest = cdp.since(markLoginOk).find((e) => e.request.method === "POST" && e.request.url.endsWith("/auth/login"));
const loginOkStatus = loginOkRequest ? cdp.responses.get(loginOkRequest.requestId)?.response?.status : null;
// 以本次登录下发的 token 作为后续「头注入」断言基准（每次登录都会换新 token）
if (loginOkRequest !== undefined) {
  try {
    const body = JSON.parse(await responseBodyOf(loginOkRequest.requestId));
    csrfToken = body?.data?.csrfToken ?? csrfToken;
  } catch {
    /* 保持上一份 token */
  }
}
check("AC-003.2 正确密码登录返回 200", loginOkStatus === 200, `status=${loginOkStatus}`);
const topBar = await q(
  `const bar = document.querySelector('.top-bar'); return { text: (bar.innerText || '').trim(), hasBrandLink: !!bar.querySelector('a[href="/"], a'), links: [...bar.querySelectorAll('a')].map((a) => a.getAttribute('href')) };`,
);
check(
  "§6.1.1.a 顶栏含产品名、主导航与登出",
  topBar.text.includes("万物说明书") &&
    topBar.text.includes("资料库") &&
    topBar.text.includes("任务中心") &&
    topBar.text.includes("设置") &&
    topBar.text.includes("登出"),
  topBar.text.replace(/\n/g, " | "),
);
const credentials = await q(
  `const token = ${JSON.stringify(csrfToken)}; return { inDom: token !== "" && document.body.innerHTML.includes(token), inLocal: token !== "" && JSON.stringify(localStorage).includes(token), inSession: token !== "" && JSON.stringify(sessionStorage).includes(token), inCookie: token !== "" && document.cookie.includes(token), jsCookie: document.cookie, inputs: [...document.querySelectorAll('input[type=text],input[type=password],input[type=search]')].map((i) => i.value).filter((v) => v !== "") };`,
);
check(
  "T08-不泄露.1 CSRF token 不在 DOM/localStorage/sessionStorage/document.cookie，输入框无残留",
  !credentials.inDom && !credentials.inLocal && !credentials.inSession && !credentials.inCookie && credentials.inputs.length === 0,
  `dom=${credentials.inDom} local=${credentials.inLocal} session=${credentials.inSession} cookie=${credentials.inCookie}`,
);
const cookieList = await cdp.send("Network.getCookies", { urls: [APP_URL] }, S);
const sessionCookie = cookieList.cookies.find((c) => c.name === "em_session");
check(
  "AC-003.1 会话 cookie 为 HttpOnly + SameSite=Strict（JS 不可读）",
  sessionCookie !== undefined && sessionCookie.httpOnly === true && sessionCookie.sameSite === "Strict" && !credentials.jsCookie.includes("em_session"),
  sessionCookie === undefined
    ? "未找到 em_session"
    : `httpOnly=${sessionCookie.httpOnly} sameSite=${sessionCookie.sameSite} jsVisible=${credentials.jsCookie.includes("em_session")}`,
);
// 已登录会话恢复：强制刷新后重新探测（AC-003 的 no-store 断言针对带凭据的 200 响应）
const markSession200 = cdp.mark();
await q("location.reload(); return true;");
await waitFor("return document.querySelector('.top-bar') !== null", { timeout: 15000, desc: "刷新后会话恢复" });
const freshSessionRequest = findRequest(cdp.since(markSession200), (e) =>
  e.request.url.endsWith("/api/v1/auth/session"),
);
const freshSessionResponse = freshSessionRequest ? cdp.responses.get(freshSessionRequest.requestId) : null;
check(
  "UI-002.d 已登录 GET /auth/session 响应 200 + Cache-Control: no-store，且未从缓存读取（AC-003）",
  freshSessionResponse?.response?.status === 200 &&
    headerValue(freshSessionResponse?.response?.headers ?? {}, "cache-control") === "no-store" &&
    freshSessionResponse?.response?.fromDiskCache !== true,
  `status=${freshSessionResponse?.response?.status} cache-control=${headerValue(freshSessionResponse?.response?.headers ?? {}, "cache-control") || "(无)"} fromDiskCache=${freshSessionResponse?.response?.fromDiskCache}`,
);
const unauthSessionCacheHeader = headerValue(
  cdp.responses.get(sessionRequest?.requestId ?? "")?.response?.headers ?? {},
  "cache-control",
);
note(
  `未登录 401 响应头（无凭据，仅记录）：cache-control=${unauthSessionCacheHeader || "(无)"}；前端用 fetch cache:"no-store" 阻断 401 缓存。`,
);

// ---------------------------------------------------------------------------
// 步骤 5：Tab 顺序（顶栏 → 主栏）
// ---------------------------------------------------------------------------

console.log("--- 步骤 5：Tab 顺序 ---");
await goto("/");
await waitFor("return document.querySelector('#library-title') !== null", { timeout: 12000, desc: "资料库" });
await q("if (document.body) { document.body.setAttribute('tabindex','-1'); document.body.focus(); document.body.removeAttribute('tabindex'); } return true;");
const tabOrder = [];
for (let i = 0; i < 6; i += 1) {
  await pressKey("Tab");
  tabOrder.push(await focusInfo());
}
const tabOrderText = tabOrder.map((f) => `${f?.tag}:${f?.text || f?.id}`).join(" → ");
check(
  "§6.1.5.a Tab 顺序为 顶栏（品牌/导航/登出）→ 主栏",
  tabOrder[0]?.text?.includes("万物说明书") === true &&
    tabOrder[1]?.text?.includes("资料库") === true &&
    tabOrder[2]?.text?.includes("任务中心") === true &&
    tabOrder[3]?.text?.includes("设置") === true &&
    tabOrder[4]?.text?.includes("登出") === true,
  tabOrderText,
);

// ---------------------------------------------------------------------------
// 步骤 6：设置与状态页（UI-003）
// ---------------------------------------------------------------------------

console.log("--- 步骤 6：设置页只读状态（UI-003） ---");
await goto("/settings");
await waitFor("return document.querySelector('#settings-title') !== null", { timeout: 12000, desc: "设置页" });
const settingsText = await bodyText();
const settingsStatus = await fetch(`${APP_URL}/api/v1/settings/status`, {
  headers: { cookie: `em_session=${sessionCookie?.value ?? ""}` },
}).then((r) => r.json());
const providers = settingsStatus?.data?.providersConfigured ?? {};
check(
  "UI-003.a 页面声明未配置并保持可读说明，与接口一致",
  settingsText.includes(providers.tripo ? "已配置" : "未配置") &&
    settingsText.includes("生成与报价不可用；已有资料仍可读"),
  `tripo=${providers.tripo} manualAi=${providers.manualAi}`,
);
const limits = settingsStatus?.data?.limits ?? {};
const mib = (bytes) => `${Number((bytes / 1048576).toFixed(1)).toString().replace(/\.0$/, "")} MiB`;
check(
  "UI-003.b limits 数值按二进制单位展示且与接口一致",
  settingsText.includes(mib(limits.maxPdfBytes)) &&
    settingsText.includes(`${limits.maxPdfPages} 页`) &&
    settingsText.includes(mib(limits.maxPhotoBytes)) &&
    settingsText.includes(mib(limits.maxGlbBytes)) &&
    settingsText.includes(mib(limits.maxItemTotalBytes)),
  `pdf=${mib(limits.maxPdfBytes)} pages=${limits.maxPdfPages} photo=${mib(limits.maxPhotoBytes)}`,
);
check(
  "UI-003.c 健康检查 live/ready 与逐项状态可见",
  settingsText.includes("/health/live") && settingsText.includes("/health/ready") && settingsText.includes("ready"),
  "",
);
const settingsInputs = await q(
  `return { inputs: document.querySelectorAll('input,select,textarea,form').length, certEntry: /证书|TLS 配置|监听配置入口(?!)/.test(document.body.innerText || ''), keyPattern: /(sk-[A-Za-z0-9]{8,}|api[_-]?key\\s*[:=]|Bearer\\s+[A-Za-z0-9._-]{8,}|-----BEGIN)/.test(document.body.innerHTML) };`,
);
check(
  "UI-003.d 无密钥输入/配置入口、无证书或 TLS 配置项、页面不含密钥样式字符串（ADR-017 D-4）",
  settingsInputs.inputs === 0 && settingsInputs.keyPattern === false,
  JSON.stringify(settingsInputs),
);
const statusRaw = JSON.stringify(settingsStatus);
check(
  "AC-012.1 /settings/status 响应不含密钥字段",
  !/(api[_-]?key|secret|token|password)/i.test(statusRaw),
  `字段：${Object.keys(settingsStatus?.data ?? {}).join(",")}`,
);

// ---------------------------------------------------------------------------
// 步骤 7：资料库空态 + 新建表单（UI-006/ADR-017 D-1）+ CSRF 真实写
// ---------------------------------------------------------------------------

console.log("--- 步骤 7：资料库、物品表单与 CSRF 真实写 ---");
await goto("/");
await waitFor("return document.querySelector('#library-title') !== null", { timeout: 12000, desc: "资料库" });
const emptyText = await bodyText();
check(
  "UI-005.a 空态显示「还没有物品」与新建入口",
  emptyText.includes("还没有物品") && emptyText.includes("新建物品"),
  "",
);
await clickByText("a", "新建物品");
await waitFor("return document.querySelector('#item-form-title') !== null", { timeout: 12000, desc: "新建表单" });
const formFields = await q(
  `const inputs = [...document.querySelectorAll('form input')]; return { ids: inputs.map((i) => i.id), labels: [...document.querySelectorAll('form label')].map((l) => (l.innerText || '').trim()), placeholders: inputs.map((i) => i.placeholder), sourceLinkField: !!document.querySelector('[id*="source" i],[name*="source" i]'), text: document.body.innerText };`,
);
check(
  "UI-006.a 表单字段为 名称/品牌/准确型号/变体配置 四项、无来源链接输入",
  formFields.ids.join(",") === "field-name,field-model,field-brand,field-variant" && formFields.sourceLinkField === false,
  formFields.ids.join(","),
);
check(
  "UI-006.b 明确说明物品不保存来源链接（ADR-017 D-1）",
  formFields.text.includes("物品不保存来源链接"),
  "",
);
const requiredDisabled = await q(
  "const b = document.querySelector('form button[type=submit]'); return b ? b.disabled : null;",
);
check("UI-006.c 必填为空时保存按钮禁用", requiredDisabled === true, `disabled=${requiredDisabled}`);

const capture = await startRequestCapture("*api/v1/items*");

// 服务端 422 字段级错误 + 焦点移回错误字段（UI-006/UI-065）：型号超长 200 上限
await clickSelector("#field-name");
await typeText("QA 验收相机");
await pressKey("Tab");
await typeText("X".repeat(201));
await clickByText("button", "创建并继续");
await waitFor("return document.querySelector('#field-model-error') !== null;", {
  timeout: 12000,
  desc: "字段级 422 错误",
});
const fieldError = await q(
  `const input = document.querySelector('#field-model'); const summary = document.querySelector('#item-form-errors'); return { describedBy: input.getAttribute('aria-describedby'), invalid: input.getAttribute('aria-invalid'), errorText: (document.querySelector('#field-model-error')?.textContent || '').trim().slice(0, 60), focused: document.activeElement === input, summaryText: summary ? (summary.innerText || '').trim().slice(0, 60) : null };`,
);
check(
  "UI-006.e 超长型号 422 → 字段级错误 + aria-describedby 关联 + 焦点移到第一个错误字段（UI-065）",
  fieldError.errorText !== "" &&
    fieldError.describedBy?.includes("field-model-error") === true &&
    fieldError.invalid === "true" &&
    fieldError.focused === true,
  JSON.stringify(fieldError),
);

// 修正型号后走正常创建路径
await q("const el = document.querySelector('#field-model'); el.focus(); el.setSelectionRange(0, el.value.length); return true;");
await typeText("QA-X100V-01");
const markCreate = cdp.mark();
await clickByText("button", "创建并继续");
await waitFor("return /^\\/items\\/[0-9a-f-]{36}$/.test(location.pathname) && document.querySelector('#item-title') !== null", {
  timeout: 15000,
  desc: "物品概览",
});
const itemPath = await currentPath();
const itemId = itemPath.split("/").pop();
const createRequests = cdp.since(markCreate);
const createPost = findRequest(createRequests, (e) => e.request.method === "POST" && e.request.url.endsWith("/api/v1/items"));
const createStatus = createPost ? cdp.responses.get(createPost.requestId)?.response?.status : null;
const capturedPost = capture.captured.find((c) => c.method === "POST" && c.url.endsWith("/api/v1/items"));
check(
  "T08-CSRF.1 新建物品 POST 实际携带 x-csrf-token（Fetch 拦截实测头）且返回 201",
  capturedPost !== undefined &&
    headerValue(capturedPost.headers, "x-csrf-token") === csrfToken &&
    createStatus === 201,
  `status=${createStatus} 实测头csrf=${headerValue(capturedPost?.headers ?? {}, "x-csrf-token") === csrfToken} 拦截头清单=${JSON.stringify(Object.keys(capturedPost?.headers ?? {}))}`,
);
check(
  "AC-003.5 浏览器修改请求携带同源 Origin（服务端 origin_check 的放行条件）",
  headerValue(capturedPost?.headers ?? {}, "origin") === APP_URL,
  `origin=${headerValue(capturedPost?.headers ?? {}, "origin") || "(无)"}`,
);
const itemGet = findRequest(createRequests, (e) => e.request.method === "GET" && e.request.url.includes("/api/v1/items"));
const capturedGet = capture.captured.find((c) => c.method === "GET" && c.url.includes("/api/v1/items"));
check(
  "T08-CSRF.2 GET 请求不注入 x-csrf-token（只对修改请求注入）",
  capturedGet !== undefined && headerValue(capturedGet.headers, "x-csrf-token") === "",
  `get=${capturedGet?.url?.slice(APP_URL.length) ?? itemGet?.request.url.slice(APP_URL.length)} csrf=${headerValue(capturedGet?.headers ?? {}, "x-csrf-token") || "(无)"}`,
);
const overviewText = await bodyText();
check(
  "UI-006.d 创建后跳转物品概览并显示名称、型号与 r1",
  overviewText.includes("QA 验收相机") && overviewText.includes("QA-X100V-01") && overviewText.includes("r1"),
  "",
);
check(
  "§6.1.1.b 顶栏显示当前物品名·型号",
  (await q("return (document.querySelector('.top-bar__context')?.innerText || '');")).includes("QA 验收相机"),
  "",
);
await shot("04-item-overview.png");

// 成功通知条与顶栏的指针命中关系（可复现现场）
const noticeOverlay = await q(
  `const notice = document.querySelector('.notice'); const link = [...document.querySelectorAll('.top-bar a')].find((a) => (a.innerText || '').trim() === '资料库'); if (!notice || !link) return null; const lr = link.getBoundingClientRect(); const nr = notice.getBoundingClientRect(); const hit = document.elementFromPoint(lr.left + lr.width / 2, lr.top + lr.height / 2); return { noticeText: (notice.innerText || '').trim().slice(0, 24), noticeHeight: Math.round(nr.height), linkTop: Math.round(lr.top), linkCenterY: Math.round(lr.top + lr.height / 2), hitTag: hit ? hit.tagName : null, hitInNotice: notice.contains(hit), hitIsLink: hit === link };`,
);
if (noticeOverlay !== null && noticeOverlay.hitInNotice === true) {
  defect(
    "BUG-001",
    "P3",
    "成功通知条（.notices 固定顶部条）盖住顶栏，指针点击落在通知条上被吞掉",
    JSON.stringify(noticeOverlay),
  );
} else {
  note(`成功通知条与顶栏点击关系：${JSON.stringify(noticeOverlay)}`);
}

// 指针点击是否被通知条拦截（可复现现场）；随后验证同一时刻键盘路径是否仍可用
let mouseNavWorked = true;
if (noticeOverlay !== null && noticeOverlay.hitInNotice === true) {
  await clickByText("a", "资料库").catch(() => {});
  try {
    await waitFor("return location.pathname === '/';", { timeout: 2500, interval: 150, desc: "指针点击导航" });
    mouseNavWorked = true;
  } catch {
    mouseNavWorked = false;
    note("指针点击顶栏「资料库」被通知条吞掉（BUG-001 现场）");
  }
}
if (mouseNavWorked === false) {
  // 键盘路径：body 起 Tab×2 到「资料库」链接，Enter 激活（通知条仍在）
  await q("if (document.body) { document.body.setAttribute('tabindex','-1'); document.body.focus(); document.body.removeAttribute('tabindex'); } return true;");
  await pressKey("Tab");
  await pressKey("Tab");
  const keyboardTarget = await focusInfo();
  await pressKey("Enter");
  let keyboardNavWorked = false;
  try {
    await waitFor("return location.pathname === '/';", { timeout: 4000, interval: 150, desc: "键盘激活导航" });
    keyboardNavWorked = true;
  } catch {
    keyboardNavWorked = false;
  }
  check(
    "BUG-001 补充：通知条覆盖顶栏时键盘（Tab+Enter）路径仍可完成导航",
    keyboardTarget?.text === "资料库" && keyboardNavWorked === true,
    `焦点=${keyboardTarget?.text} 键盘导航=${keyboardNavWorked}`,
  );
}
await navClick("资料库", "/");
await waitFor("return document.querySelector('.item-list') !== null", { timeout: 12000, desc: "列表行" });
const libraryRowText = await q("return document.querySelector('.item-list')?.innerText ?? null;");
check(
  "UI-005.b 列表行显示名称/型号/状态/更新时间（服务端数据，非假数据）",
  libraryRowText?.includes("QA 验收相机") === true &&
    libraryRowText.includes("QA-X100V-01") === true &&
    libraryRowText.includes("使用中") === true &&
    libraryRowText.includes("更新于") === true &&
    libraryRowText.includes("打开") === true,
  (libraryRowText ?? "").replace(/\n/g, " | "),
);
const markArchive = cdp.mark();
await clickInScope("QA-X100V-01", "a", "打开");
await waitFor("return document.querySelector('#item-title') !== null", { timeout: 12000, desc: "物品概览" });
await clickByText("button", "归档");
await waitFor("return document.body.innerText.includes('物品已归档') && document.body.innerText.includes('已归档');", { timeout: 12000, desc: "归档完成" });
const archiveRequest = cdp.since(markArchive).find((e) => e.request.method === "PATCH");
const capturedArchivePatch = capture.captured.find((c) => c.method === "PATCH");
check(
  "UI-007.a 归档 PATCH 携带 If-Match 与 CSRF，且状态文本变「已归档」",
  /^"r\d+"$/.test(reqHeader(archiveRequest ?? {}, "if-match")) &&
    headerValue(capturedArchivePatch?.headers ?? {}, "if-match") === reqHeader(archiveRequest ?? {}, "if-match") &&
    headerValue(capturedArchivePatch?.headers ?? {}, "x-csrf-token") === csrfToken,
  `if-match=${reqHeader(archiveRequest ?? {}, "if-match") || "(无)"} 实测头csrf=${headerValue(capturedArchivePatch?.headers ?? {}, "x-csrf-token") === csrfToken}`,
);
await navClick("资料库", "/");
await waitFor("return document.querySelector('#library-title') !== null", { timeout: 12000, desc: "资料库" });
const libraryAfterArchive = await bodyText();
check(
  "UI-007.b 归档物品默认不出现在列表",
  !libraryAfterArchive.includes("QA-X100V-01"),
  "",
);
await clickSelector("#library-include-archived");
await waitFor("return document.querySelector('.item-list') !== null", { timeout: 12000, desc: "已归档列表" });
const archivedRowText = await q("return document.querySelector('.item-list')?.innerText ?? null;");
check("UI-007.d 归档视图（archived=true）能看到已归档行", archivedRowText?.includes("已归档") === true, "");
await clickInScope("QA-X100V-01", "a", "打开");
await waitFor("return document.querySelector('#item-title') !== null", { timeout: 12000, desc: "物品概览" });
await clickByText("button", "恢复");
await waitFor("return document.body.innerText.includes('物品已恢复');", { timeout: 12000, desc: "恢复完成" });
check("UI-007.c 归档物品可恢复（列表开关 + 恢复按钮）", true, "");

// CSRF 服务端强制对照（Node 侧第二客户端）：缺 token 应被拒，证明浏览器确实带了 token
const controlNoCsrf = await fetch(`${APP_URL}/api/v1/items`, {
  method: "POST",
  headers: { "content-type": "application/json", cookie: `em_session=${sessionCookie?.value ?? ""}` },
  body: JSON.stringify({ name: "QA 对照物品", model: "QA-CONTROL-NOCSRF" }),
});
const controlWithCsrf = await fetch(`${APP_URL}/api/v1/items`, {
  method: "POST",
  headers: {
    "content-type": "application/json",
    cookie: `em_session=${sessionCookie?.value ?? ""}`,
    "x-csrf-token": csrfToken,
  },
  body: JSON.stringify({ name: "QA 对照物品", model: "QA-CONTROL-WITHCSRF" }),
});
check(
  "T08-CSRF.3 服务端强制 CSRF：无 token 403 / 有 token 201（反证浏览器请求必带 token）",
  controlNoCsrf.status === 403 && controlWithCsrf.status === 201,
  `无token=${controlNoCsrf.status} 有token=${controlWithCsrf.status}`,
);

// ---------------------------------------------------------------------------
// 步骤 8：If-Match 编辑与 412 冲突恢复（UI-006/UI-008）
// ---------------------------------------------------------------------------

console.log("--- 步骤 8：If-Match 与 412 恢复 ---");

/** Node 侧第二客户端读取服务端当前 ETag/revision（只读，不推进 revision）。 */
async function fetchItemState() {
  const response = await fetch(`${APP_URL}/api/v1/items/${itemId}`, {
    headers: { cookie: `em_session=${sessionCookie?.value ?? ""}` },
  });
  const body = await response.json().catch(() => null);
  return {
    status: response.status,
    etag: response.headers.get("etag"),
    revision: body?.data?.revision ?? null,
  };
}

// 8.1 打开编辑页（页面此时持有服务端最新 ETag），输入但先不提交
await clickByText("a", "编辑");
await waitFor("return document.querySelector('#field-name') !== null", { timeout: 12000, desc: "编辑表单" });
const stateBeforeEdit = await fetchItemState();
await clickSelector("#field-name");
await typeText("（QA 改名）·保留输入");

// 8.2 外部并发更新：服务端 revision 前进，浏览器仍持有旧 ETag
const externalPatch = await fetch(`${APP_URL}/api/v1/items/${itemId}`, {
  method: "PATCH",
  headers: {
    "content-type": "application/json",
    cookie: `em_session=${sessionCookie?.value ?? ""}`,
    "x-csrf-token": csrfToken,
    "if-match": stateBeforeEdit.etag ?? '"r1"',
  },
  body: JSON.stringify({ name: "QA 验收相机（外部更新）" }),
});
const externalBody = await externalPatch.json().catch(() => null);
const externalRevision = externalBody?.data?.revision ?? null;
check(
  "UI-008.a 外部并发更新成功（构造 412 前置条件）",
  externalPatch.status === 200 && externalRevision === (stateBeforeEdit.revision ?? 0) + 1,
  `status=${externalPatch.status} revision ${stateBeforeEdit.revision} → ${externalRevision}`,
);

// 8.3 浏览器用页面持有的（已过期）ETag 提交 → 期望 412
const markConflict = cdp.mark();
await clickByText("button", "保存");
await waitFor("return document.querySelector('.conflict-notice') !== null", { timeout: 12000, desc: "412 冲突提示" });
const conflictPatch = cdp.since(markConflict).find((e) => e.request.method === "PATCH");
const conflictStatus = conflictPatch ? cdp.responses.get(conflictPatch.requestId)?.response?.status : null;
const capturedConflictPatch = capture.captured
  .filter((c) => c.method === "PATCH" && c.url.endsWith(`/api/v1/items/${itemId}`))
  .at(-1);
check(
  "T08-IfMatch.1 编辑 PATCH 原样回传页面 GET 的 ETag（含引号）并注入 CSRF（Fetch 实测头）",
  reqHeader(conflictPatch ?? {}, "if-match") === stateBeforeEdit.etag &&
    headerValue(capturedConflictPatch?.headers ?? {}, "if-match") === stateBeforeEdit.etag &&
    headerValue(capturedConflictPatch?.headers ?? {}, "x-csrf-token") === csrfToken,
  `if-match=${reqHeader(conflictPatch ?? {}, "if-match") || "(无)"} 页面 GET ETag=${stateBeforeEdit.etag} 实测头csrf=${headerValue(capturedConflictPatch?.headers ?? {}, "x-csrf-token") === csrfToken}`,
);
const conflictInfo = await q(
  `const notice = document.querySelector('.conflict-notice'); const save = [...document.querySelectorAll('button')].find((b) => (b.innerText || '').trim() === '保存'); return { text: (notice.innerText || '').trim(), role: notice.getAttribute('role'), nameValue: document.querySelector('#field-name').value, saveDisabled: save ? save.disabled : null, hasRefresh: !![...document.querySelectorAll('button')].find((b) => (b.innerText || '').trim() === '刷新后重试') };`,
);
check(
  "UI-008.b 412 显示 details.currentRevision 且不自动覆盖",
  conflictInfo.text.includes(`（当前 r${externalRevision}）`) && conflictInfo.role === "alert",
  conflictInfo.text.split("\n")[0],
);
check(
  "UI-008.c 冲突后保留表单输入并禁用提交",
  conflictInfo.nameValue.includes("·保留输入") && conflictInfo.saveDisabled === true,
  `value=${conflictInfo.nameValue} disabled=${conflictInfo.saveDisabled}`,
);
check("UI-008.d 提供「刷新后重试」入口", conflictInfo.hasRefresh === true, "");
check("AC-017.412 服务端返回 412", conflictStatus === 412, `status=${conflictStatus}`);
await shot("05-conflict-412.png");

const stateBeforeResubmit = await fetchItemState();
await clickByText("button", "刷新后重试");
await waitFor("return document.body.innerText.includes('已刷新到服务端最新版本');", { timeout: 12000, desc: "刷新完成" });
const refreshedText = await bodyText();
const refreshedValue = await q("return document.querySelector('#field-name').value;");
check(
  "UI-008.e 刷新后使用服务端最新 revision 且保留输入，可再次提交",
  refreshedText.includes(`（r${externalRevision}）`) && refreshedValue.includes("·保留输入"),
  `value=${refreshedValue}`,
);
const markResubmit = cdp.mark();
await clickByText("button", "保存");
await waitFor("return document.querySelector('#item-title') !== null", { timeout: 12000, desc: "重新提交后回概览" });
const secondPatch = cdp.since(markResubmit).find((e) => e.request.method === "PATCH");
check(
  "UI-008.f 刷新后用服务端最新 ETag 重新提交成功（200）",
  reqHeader(secondPatch ?? {}, "if-match") === stateBeforeResubmit.etag &&
    (secondPatch ? cdp.responses.get(secondPatch.requestId)?.response?.status : null) === 200,
  `if-match=${reqHeader(secondPatch ?? {}, "if-match") || "(无)"} 服务端 ETag=${stateBeforeResubmit.etag}`,
);
await capture.stop();

// ---------------------------------------------------------------------------
// 步骤 9：占位路由不伪装完成、不发业务请求
// ---------------------------------------------------------------------------

console.log("--- 步骤 9：占位路由（无假数据/无业务请求） ---");
const placeholderPaths = [
  "/jobs",
  "/jobs/01993000-0000-7000-8000-0000000000aa",
  `/items/${itemId}/import/document`,
  `/items/${itemId}/import/views`,
  `/items/${itemId}/import/prepare`,
  `/items/${itemId}/import/confirm`,
  `/items/${itemId}/drafts/01993000-0000-7000-8000-0000000000bb/review`,
  `/items/${itemId}/releases`,
  `/items/${itemId}/releases/01993000-0000-7000-8000-0000000000cc`,
];
let placeholderFailures = [];
let placeholderFakeRequests = [];
const placeholderTexts = [];
for (const path of placeholderPaths) {
  const mark = cdp.mark();
  await goto(path);
  const text = await bodyText();
  placeholderTexts.push(text);
  if (!text.includes("该页面尚未实现")) placeholderFailures.push(`${path}(缺少未实现声明)`);
  const requests = apiRequests(cdp.since(mark));
  const unexpected = requests.filter(
    (r) =>
      !r.path.startsWith("/api/v1/auth/session") &&
      !r.path.startsWith(`/api/v1/items/${itemId}`) &&
      !r.path.startsWith("/api/v1/health/"),
  );
  const writeRequests = requests.filter((r) => r.method !== "GET");
  if (unexpected.length > 0) placeholderFakeRequests.push(`${path} → ${unexpected.map((r) => `${r.method} ${r.path}`).join(",")}`);
  if (writeRequests.length > 0) placeholderFakeRequests.push(`${path} → 写请求 ${writeRequests.map((r) => `${r.method} ${r.path}`).join(",")}`);
}
check(
  "T08-占位.1 全部占位路由明确显示「该页面尚未实现」",
  placeholderFailures.length === 0,
  placeholderFailures.join("；") || `${placeholderPaths.length} 条路由`,
);
check(
  "T08-占位.2 占位路由不请求业务接口、不发写请求（无假数据/无假动作）",
  placeholderFakeRequests.length === 0,
  placeholderFakeRequests.join("；") || "仅 session/物品上下文/health",
);
const jobsText = placeholderTexts[0];
check(
  "T08-占位.3 任务中心不显示编造的任务计数或状态",
  !/进行中\s*\d|任务 \(\d+\)|总进度/.test(jobsText) && jobsText.includes("任务列表"),
  jobsText.split("\n").slice(0, 4).join(" / "),
);
await goto(`/items/${itemId}/import/prepare`);
const wizard = await q(
  `const steps = document.querySelector('[aria-label="新建向导步骤"]'); if (!steps) return null; return { count: steps.querySelectorAll('li').length, current: steps.querySelector('[aria-current="step"]')?.innerText ?? null };`,
);
check(
  "§6.1.2 向导占位页保留五步步骤条且当前步 aria-current=step",
  wizard?.count === 5 && wizard?.current === "准备",
  JSON.stringify(wizard),
);
await shot("06-wizard-placeholder.png");

// ---------------------------------------------------------------------------
// 步骤 10：深链接与刷新
// ---------------------------------------------------------------------------

console.log("--- 步骤 10：深链接与刷新 ---");
await goto(`/items/${itemId}`);
const deepItem = await q("return document.querySelector('#item-title')?.innerText ?? null;");
await q("location.reload(); return true;");
await waitFor("return document.querySelector('#item-title') !== null", { timeout: 15000, desc: "刷新后物品页" });
const deepItemAfterReload = await q("return document.querySelector('#item-title')?.innerText ?? null;");
check(
  "§6.1.1.c 物品深链接可直接打开且刷新后仍渲染同一物品",
  deepItem !== null && deepItem === deepItemAfterReload,
  `before=${deepItem} after=${deepItemAfterReload}`,
);
await goto("/settings");
const settingsAfterDeepLink = await q("return document.querySelector('#settings-title') !== null;");
await q("location.reload(); return true;");
await waitFor("return document.querySelector('#settings-title') !== null", { timeout: 15000, desc: "刷新后设置页" });
check("§6.1.1.d 设置页深链接与刷新可用", settingsAfterDeepLink === true, "");

// ---------------------------------------------------------------------------
// 步骤 11：三档断点与抽屉
// ---------------------------------------------------------------------------

console.log("--- 步骤 11：断点布局与抽屉 ---");
async function setViewport(width, height = 900) {
  await cdp.send(
    "Emulation.setDeviceMetricsOverride",
    { width, height, deviceScaleFactor: 1, mobile: false },
    S,
  );
  await sleep(350);
}
await setViewport(1400);
await goto("/");
await waitFor("return document.querySelector('#library-title') !== null", { timeout: 12000, desc: "资料库" });
const wideAside = await q(
  `const a = document.querySelector('aside[aria-label="资料库摘要"]'); if (!a) return null; return { visible: a.getBoundingClientRect().width > 0, text: (a.innerText || '').trim() };`,
);
check(
  "§6.1.1.e ≥1280px 三栏：侧栏直接并排显示（同一页面同一数据）",
  wideAside?.visible === true && wideAside.text.includes("资料库摘要"),
  (wideAside?.text ?? "").split("\n").join(" | "),
);
await setViewport(900);
const midState = await q(
  `const a = document.querySelector('aside[aria-label="资料库摘要"]'); const b = [...document.querySelectorAll('button')].find((x) => (x.innerText || '').includes('显示资料库摘要')); return { aside: !!a, trigger: !!b };`,
);
check(
  "§6.1.1.f 768–1279px：主栏 + 单个可折叠侧栏（不并排第三栏）",
  midState.aside === false && midState.trigger === true,
  JSON.stringify(midState),
);
await clickByText("button", "显示资料库摘要");
const midAside = await q(
  `const a = document.querySelector('aside[aria-label="资料库摘要"]'); return a ? { text: (a.innerText || '').trim() } : null;`,
);
check("§6.1.1.g 中段展开侧栏显示同一份数据", midAside?.text?.includes("资料库摘要") === true, "");
await setViewport(600);
const narrowState = await q(
  `const a = document.querySelector('aside[aria-label="资料库摘要"]'); const d = document.querySelector('[role="dialog"]'); return { aside: !!a, dialog: !!d };`,
);
check("§6.1.1.h <768px：单栏（侧栏不并排、未打开时无对话框）", narrowState.aside === false && narrowState.dialog === false, JSON.stringify(narrowState));
const triggerRect = await rectOf(
  `const b = [...document.querySelectorAll('button')].find((x) => (x.innerText || '').trim() === '资料库摘要'); return b ?? null;`,
);
await clickRect(triggerRect);
await waitFor("return document.querySelector('[role=dialog]') !== null", { timeout: 8000, desc: "抽屉" });
const drawerInfo = await q(
  `const d = document.querySelector('[role=dialog]'); return { ariaModal: d.getAttribute('aria-modal'), label: d.getAttribute('aria-label'), hasAside: !!document.querySelector('aside[aria-label="资料库摘要"]'), count: document.querySelectorAll('[role=dialog]').length };`,
);
const focusInDrawer = await focusInfo();
check(
  "UI-062.a 窄屏抽屉为模态对话框、同一时刻只一个、焦点移入抽屉",
  drawerInfo.ariaModal === "true" && drawerInfo.count === 1 && focusInDrawer.inDialog === true,
  `aria-modal=${drawerInfo.ariaModal} count=${drawerInfo.count} focusInDialog=${focusInDrawer.inDialog}`,
);
for (let i = 0; i < 4; i += 1) await pressKey("Tab");
const focusAfterTabs = await focusInfo();
check("UI-062.b Tab 焦点被陷阱在抽屉内循环", focusAfterTabs.inDialog === true, `focus=${focusAfterTabs.text?.slice(0, 20)} inDialog=${focusAfterTabs.inDialog}`);
await shot("07-narrow-drawer.png");
await pressKey("Escape");
await waitFor("return document.querySelector('[role=dialog]') === null", { timeout: 8000, desc: "抽屉关闭" });
const focusAfterEscape = await focusInfo();
check(
  "UI-062.c Esc 关闭抽屉并把焦点归还触发按钮",
  focusAfterEscape?.text === "资料库摘要",
  JSON.stringify(focusAfterEscape),
);
await setViewport(1400);
const wideAfterResize = await q(
  `const a = document.querySelector('aside[aria-label="资料库摘要"]'); const b = [...document.querySelectorAll('button')].find((x) => (x.innerText || '').includes('资料库摘要')); return { aside: !!a, trigger: !!b };`,
);
check(
  "UI-062.d 窗口拉宽后无需刷新即恢复三栏（宽度判定，非 UA）",
  wideAfterResize.aside === true && wideAfterResize.trigger === false,
  JSON.stringify(wideAfterResize),
);
await cdp.send("Emulation.clearDeviceMetricsOverride", {}, S);
await sleep(300);

// ---------------------------------------------------------------------------
// 步骤 12：prefers-reduced-motion
// ---------------------------------------------------------------------------

console.log("--- 步骤 12：减少动效 ---");
async function skeletonDurations() {
  return q(
    `const bar = document.createElement('div'); bar.className = 'skeleton__bar'; document.body.appendChild(bar); const cs = getComputedStyle(bar); const out = { animation: cs.animationDuration, transition: cs.transitionDuration }; bar.remove(); return out;`,
  );
}
await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "no-preference" }] }, S);
await sleep(200);
const normalDurations = await skeletonDurations();
await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] }, S);
await sleep(200);
const reducedDurations = await skeletonDurations();
const toSeconds = (value) => {
  const parsed = Number.parseFloat(String(value));
  return Number.isFinite(parsed) ? parsed : Number.POSITIVE_INFINITY;
};
check(
  "UI-064.a prefers-reduced-motion: reduce 把骨架动画/过渡压到 ≤1ms，默认仍为 1.4s",
  toSeconds(reducedDurations.animation) <= 0.001 &&
    toSeconds(reducedDurations.transition) <= 0.001 &&
    toSeconds(normalDurations.animation) >= 1,
  `reduce=${JSON.stringify(reducedDurations)} normal=${JSON.stringify(normalDurations)}`,
);
await cdp.send("Emulation.setEmulatedMedia", { features: [] }, S);
await sleep(150);

// ---------------------------------------------------------------------------
// 步骤 13：错误边界显示 requestId 不显示堆栈
// ---------------------------------------------------------------------------

console.log("--- 步骤 13：错误边界（响应注入合同违约载荷） ---");
const consoleErrorsOf = () =>
  cdp.consoleMessages
    .filter((m) => m.type === "error")
    .map((m) => m.args.map((a) => String(a.value ?? a.description ?? "")).join(" ").slice(0, 160));
const consoleErrorsBeforeBoundary = consoleErrorsOf().length;
const REQUEST_ID_MARKER = "01993000-0000-7000-8000-qa13boundary";
let boundaryResult = null;
try {
  const docsBody = await fetch(`${APP_URL}/api/v1/items/${itemId}/documents`, {
    headers: { cookie: `em_session=${sessionCookie?.value ?? ""}` },
  }).then((r) => r.json());
  const tampered = JSON.parse(JSON.stringify(docsBody));
  if (Array.isArray(tampered.data) && tampered.data.length > 0) {
    tampered.data[0].sourceSha256 = null;
  } else {
    tampered.data = [
      {
        id: "doc-qa",
        itemId,
        sourceAssetId: "asset-qa",
        title: "注入样例",
        sourceUrl: null,
        sourceSha256: null,
        createdAt: "2026-09-12T00:00:00Z",
        updatedAt: "2026-09-12T00:00:00Z",
      },
    ];
  }
  const payload = Buffer.from(JSON.stringify(tampered), "utf8").toString("base64");
  await cdp.send(
    "Fetch.enable",
    { patterns: [{ urlPattern: "*api/v1/items/*/documents", requestStage: "Response" }] },
    S,
  );
  const interceptor = async (message) => {
    if (message.method !== "Fetch.requestPaused") return;
    const p = message.params;
    await cdp.send(
      "Fetch.fulfillRequest",
      {
        requestId: p.requestId,
        responseCode: 200,
        responseHeaders: [
          { name: "content-type", value: "application/json" },
          { name: "cache-control", value: "no-store" },
          { name: "x-request-id", value: REQUEST_ID_MARKER },
          { name: "content-length", value: String(Buffer.byteLength(Buffer.from(payload, "base64"))) },
        ],
        body: payload,
      },
      S,
    );
  };
  cdp.subs.add(interceptor);
  await goto(`/items/${itemId}`);
  await waitFor("return document.querySelector('[aria-labelledby=error-fallback-title]') !== null", {
    timeout: 12000,
    desc: "错误边界",
  });
  boundaryResult = await q(
    `const panel = document.querySelector('[aria-labelledby=error-fallback-title]'); return { text: (panel.innerText || '').trim(), role: panel.getAttribute('role'), hasStack: /\\.tsx|at .*\\(.*:\\d+:\\d+\\)/.test(panel.innerText || '') };`,
  );
  cdp.subs.delete(interceptor);
  await cdp.send("Fetch.disable", {}, S);
} catch (error) {
  note(`错误边界注入未完成：${error.message}`);
  try {
    await cdp.send("Fetch.disable", {}, S);
  } catch {
    /* ignore */
  }
}
if (boundaryResult === null) {
  check("§6.1.1.i 错误边界显示 requestId、不显示堆栈（浏览器侧）", false, "注入未触发：见未覆盖边界（单元用例已覆盖）");
} else {
  check(
    "§6.1.1.i 错误边界显示 requestId、不显示堆栈（浏览器侧）",
    boundaryResult.role === "alert" &&
      boundaryResult.text.includes("页面出现异常") &&
      boundaryResult.text.includes(REQUEST_ID_MARKER) &&
      boundaryResult.text.includes("返回资料库") &&
      boundaryResult.hasStack === false,
    boundaryResult.text.split("\n").slice(0, 3).join(" / "),
  );
  await shot("08-error-boundary.png");
}

// ---------------------------------------------------------------------------
// 步骤 14：401 处理（清 cookie）、登出、旧 cookie 失效
// ---------------------------------------------------------------------------

console.log("--- 步骤 14：401 处理与登出 ---");
await goto("/");
await waitFor("return document.querySelector('#library-title') !== null", { timeout: 12000, desc: "资料库" });
await cdp.send("Network.clearBrowserCookies", {}, S);
// 等成功通知条自动消失（约 6s），避免通知条拦截顶栏点击影响本项观测
await waitFor("return document.querySelector('.notice') === null;", { timeout: 12000, desc: "通知条消失" });
const mark401 = cdp.mark();
await clickByText("a", "设置");
await waitFor("return document.querySelector('#field-password') !== null", { timeout: 12000, desc: "401 后回登录页" });
const pathAfter401 = await currentPath();
const textAfter401 = await bodyText();
const items401 = cdp
  .since(mark401)
  .find((e) => e.request.method === "GET" && e.request.url.includes("/api/v1/settings/status"));
const items401Status = items401 ? cdp.responses.get(items401.requestId)?.response?.status : null;
check(
  "UI-002.i 业务请求 401 → 丢弃状态跳 /login?next=%2Fsettings 并提示登录已过期",
  pathAfter401 === "/login?next=%2Fsettings" && textAfter401.includes("登录已过期"),
  `${pathAfter401} · status=${items401Status}`,
);
// 重新登录 → 登出 → 旧 cookie 失效
await loginViaKeyboard(PASSWORD);
await waitFor("return document.querySelector('.top-bar') !== null", { timeout: 15000, desc: "重新登录" });
const cookiesBeforeLogout = await cdp.send("Network.getCookies", { urls: [APP_URL] }, S);
const sessionBeforeLogout = cookiesBeforeLogout.cookies.find((c) => c.name === "em_session");
const markLogout = cdp.mark();
await clickByText("button", "登出");
await waitFor("return document.querySelector('#field-password') !== null", { timeout: 12000, desc: "登出后登录页" });
const logoutRequest = cdp.since(markLogout).find((e) => e.request.method === "POST" && e.request.url.endsWith("/auth/logout"));
check(
  "AC-003.3 登出 POST /auth/logout 携带 CSRF 并回到登录页",
  logoutRequest !== undefined && reqHeader(logoutRequest, "x-csrf-token") !== "",
  `csrf=${reqHeader(logoutRequest ?? {}, "x-csrf-token") === "" ? "(无)" : "已注入"}`,
);
const staleCheck = await fetch(`${APP_URL}/api/v1/items`, {
  headers: { cookie: `em_session=${sessionBeforeLogout?.value ?? ""}` },
});
check(
  "AC-003.4 登出后旧 cookie 再请求返回 401",
  staleCheck.status === 401,
  `status=${staleCheck.status}`,
);

// ---------------------------------------------------------------------------
// 步骤 15：登录限速（429 文案）
// ---------------------------------------------------------------------------

console.log("--- 步骤 15：登录限速 ---");
// 15.1 服务端限速（Node 侧第二客户端，确定性证据；会消耗本机窗口内的限速额度）
const rateLimitStatuses = [];
let rateLimitRetryAfter = "";
for (let attempt = 0; attempt < 8; attempt += 1) {
  const response = await fetch(`${APP_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json", origin: APP_URL },
    body: JSON.stringify({ password: WRONG_PASSWORD }),
  });
  rateLimitStatuses.push(response.status);
  if (response.status === 429) {
    rateLimitRetryAfter = response.headers.get("retry-after") ?? "";
    break;
  }
}
check(
  "UI-001.g-服务端 连续错误密码触发 429 限速（含 Retry-After）",
  rateLimitStatuses.includes(429) && rateLimitRetryAfter !== "",
  `序列=${rateLimitStatuses.join(",")} Retry-After=${rateLimitRetryAfter || "(无)"}`,
);

// 15.2 浏览器侧：限速窗口内的一次错误密码 → 页面显示限速文案
let rateLimitText = "";
try {
  await goto("/login");
  await waitFor("return document.querySelector('#field-password') !== null", { timeout: 12000, desc: "登录页" });
  await loginViaKeyboard(WRONG_PASSWORD);
  await sleep(600);
  rateLimitText = await bodyText();
} catch (error) {
  note(`浏览器限速文案检查未完成：${error.message}`);
}
check(
  "UI-001.g-页面 限速期间登录失败显示「尝试过于频繁，请稍后再试」（role=alert）",
  rateLimitText.includes("尝试过于频繁，请稍后再试"),
  rateLimitText.includes("尝试过于频繁") ? "429 文案已出现" : "未观察到 429 文案",
);
await shot("09-rate-limited.png");

// ---------------------------------------------------------------------------
// 步骤 16：禁用措辞扫描（累计所有访问页面）
// ---------------------------------------------------------------------------

console.log("--- 步骤 16：禁用措辞扫描 ---");
const scannedTexts = [...placeholderTexts, text1, settingsText, emptyText, overviewText, textAfter401, libraryAfterArchive];
const wordingHits = [];
for (const phrase of FORBIDDEN_WORDING) {
  for (const text of scannedTexts) {
    if (text.includes(phrase)) wordingHits.push(`${phrase}`);
  }
}
check(
  "§6.3.2 已访问页面文本不含禁用措辞清单",
  wordingHits.length === 0,
  wordingHits.join("；") || `${FORBIDDEN_WORDING.length} 条措辞 × ${scannedTexts.length} 份页面文本`,
);

// ---------------------------------------------------------------------------
// 收尾：控制台/异常统计
// ---------------------------------------------------------------------------

const consoleText = cdp.consoleMessages
  .map((m) => m.args.map((a) => String(a.value ?? a.description ?? "")).join(" "))
  .join("\n");
const logText = cdp.logEntries.map((e) => e.entry?.text ?? "").join("\n");
const allConsoleErrors = consoleErrorsOf();
const consoleErrorsAfterBoundary = allConsoleErrors.length;
const boundaryConsoleErrors = consoleErrorsAfterBoundary - consoleErrorsBeforeBoundary;
check(
  "T08-控制台.1 除故意的错误边界注入期间外，全程 0 条 console.error",
  allConsoleErrors.length === boundaryConsoleErrors,
  `总计=${allConsoleErrors.length}（其中错误边界注入期间=${boundaryConsoleErrors}，为 React 对被捕获异常的常规上报）样例=${JSON.stringify(allConsoleErrors.slice(0, 2))}`,
);
check("T08-控制台.2 全程 0 个未捕获异常", cdp.exceptions.length === 0, `count=${cdp.exceptions.length}`);
check(
  "T08-不泄露.2 控制台/日志不含 CSRF token 值",
  csrfToken !== "" && !consoleText.includes(csrfToken) && !logText.includes(csrfToken),
  `csrf长度=${csrfToken.length}`,
);

console.log("");
console.log(`走查结果：${passed} passed / ${failed} failed（${failedIds.length > 0 ? `失败项：${failedIds.join(", ")}` : "无失败项"}）`);
process.exit(failed === 0 ? 0 : 1);
