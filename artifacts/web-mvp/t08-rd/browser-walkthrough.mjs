#!/usr/bin/env node
/**
 * T08 浏览器联调（真实 Chrome + 真实后端 + Vite dev，通过 CDP 驱动）。
 *
 * 覆盖：
 * 1. 未登录访问 / → 会话探测 401 → 跳 /login?next=%2F（保留 next）；
 * 2. 键盘 Tab 走登录表单（焦点顺序 + 可见焦点）；
 * 3. 错误密码 → 401 文案 + 焦点移到错误摘要；
 * 4. 正确密码 → 回到 / 空态「还没有物品」；
 * 5. 真实新建物品（真实 POST + CSRF）→ 物品概览 → 资料库出现该行；
 * 6. 清除 cookie 后的会话过期处理 → 301/401 → 跳登录页并保留 next；
 * 7. 登出 → 回登录页；
 * 8. 窄屏（600px）抽屉：触发按钮 → dialog → Esc 关闭并归还焦点；
 * 9. prefers-reduced-motion: reduce 下骨架动画被关闭。
 *
 * 依赖：仅 Node 内置能力（全局 WebSocket/ fetch）。截图写入 OUT_DIR。
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const CDP_PORT = process.env.CDP_PORT ?? "9333";
const APP_URL = process.env.APP_URL ?? "http://127.0.0.1:5173";
const OUT_DIR = process.env.OUT_DIR ?? ".";
const PASSWORD = process.env.EM_PASSWORD ?? "t08-browser-password";

mkdirSync(OUT_DIR, { recursive: true });

const checks = [];
let failed = 0;

function check(name, ok, detail = "") {
  checks.push({ name, ok, detail });
  const mark = ok ? "PASS" : "FAIL";
  if (!ok) {
    failed += 1;
  }
  console.log(`[${mark}] ${name}${detail === "" ? "" : ` — ${detail}`}`);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

class Cdp {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    this.sessionId = null;
    this.consoleErrors = [];
    this.exceptions = [];
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== undefined && this.pending.has(message.id)) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) {
          reject(new Error(`${message.error.message} (${message.error.code})`));
        } else {
          resolve(message.result);
        }
        return;
      }
      if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
        this.consoleErrors.push(
          message.params.args.map((arg) => arg.value ?? arg.description ?? "").join(" "),
        );
      }
      if (message.method === "Runtime.exceptionThrown") {
        this.exceptions.push(message.params.exceptionDetails.text ?? "unknown exception");
      }
    });
  }

  static async connect(port) {
    const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
    const socket = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    return new Cdp(socket);
  }

  send(method, params = {}, withSession = true) {
    const id = this.nextId++;
    const payload = { id, method, params };
    if (withSession && this.sessionId !== null) {
      payload.sessionId = this.sessionId;
    }
    this.socket.send(JSON.stringify(payload));
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new Error(`CDP 超时：${method}`));
        }
      }, 20000);
    });
  }
}

const cdp = await Cdp.connect(CDP_PORT);
const page = await cdp.send("Target.createTarget", { url: "about:blank" }, false);
const attached = await cdp.send(
  "Target.attachToTarget",
  { targetId: page.targetId, flatten: true },
  false,
);
cdp.sessionId = attached.sessionId;

await cdp.send("Page.enable", {}, true);
await cdp.send("Runtime.enable", {}, true);
await cdp.send("Network.enable", {}, true);
await cdp.send("Emulation.setDeviceMetricsOverride", {
  width: 1400,
  height: 900,
  deviceScaleFactor: 1,
  mobile: false,
});

async function evaluate(expression) {
  const result = await cdp.send(
    "Runtime.evaluate",
    { expression, returnByValue: true, awaitPromise: true },
    true,
  );
  if (result.exceptionDetails) {
    throw new Error(`页面执行异常：${result.exceptionDetails.text ?? ""}`);
  }
  return result.result.value;
}

async function waitFor(description, expression, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    last = await evaluate(expression);
    if (last) {
      return last;
    }
    await sleep(120);
  }
  throw new Error(`等待超时：${description}（最后取值 ${JSON.stringify(last)}）`);
}

async function screenshot(name) {
  const result = await cdp.send(
    "Page.captureScreenshot",
    { format: "png", captureBeyondViewport: false },
    true,
  );
  const path = join(OUT_DIR, name);
  writeFileSync(path, Buffer.from(result.data, "base64"));
  console.log(`[截图] ${path}`);
  return path;
}

async function pressKey(key, code, keyCode, modifiers = 0, text = undefined) {
  const base = {
    key,
    code,
    windowsVirtualKeyCode: keyCode,
    nativeVirtualKeyCode: keyCode,
    modifiers,
  };
  // 有文本的按键用 keyDown（触发浏览器默认动作，如表单隐式提交）；
  // 无文本的按键（Tab/Escape）用 rawKeyDown。
  await cdp.send(
    "Input.dispatchKeyEvent",
    text === undefined ? { ...base, type: "rawKeyDown" } : { ...base, type: "keyDown", text, unmodifiedText: text },
    true,
  );
  await cdp.send("Input.dispatchKeyEvent", { ...base, type: "keyUp" }, true);
}

const pressTab = () => pressKey("Tab", "Tab", 9);
const pressEscape = () => pressKey("Escape", "Escape", 27);
const pressEnter = () => pressKey("Enter", "Enter", 13, 0, "\r");

async function typeText(text) {
  for (const character of text) {
    await cdp.send(
      "Input.dispatchKeyEvent",
      { type: "keyDown", key: character, text: character, unmodifiedText: character },
      true,
    );
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: character }, true);
  }
}

/** 清空当前输入框（多次退格；比重依赖平台快捷键的选择全选更稳）。 */
async function clearFocusedField() {
  for (let index = 0; index < 40; index += 1) {
    await pressKey("Backspace", "Backspace", 8);
  }
}

async function boxOf(selector) {
  return evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) return null;
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  })()`);
}

async function clickSelector(selector) {
  const box = await boxOf(selector);
  if (box === null) {
    throw new Error(`找不到可点击元素：${selector}`);
  }
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: box.x, y: box.y }, true);
  await cdp.send(
    "Input.dispatchMouseEvent",
    { type: "mousePressed", x: box.x, y: box.y, button: "left", clickCount: 1 },
    true,
  );
  await cdp.send(
    "Input.dispatchMouseEvent",
    { type: "mouseReleased", x: box.x, y: box.y, button: "left", clickCount: 1 },
    true,
  );
}

async function markByText(selector, text) {
  return evaluate(`(() => {
    const nodes = [...document.querySelectorAll(${JSON.stringify(selector)})];
    const el = nodes.find((node) => (node.textContent ?? "").trim().includes(${JSON.stringify(text)}));
    if (!el) return false;
    document.querySelectorAll("[data-t08probe]").forEach((n) => n.removeAttribute("data-t08probe"));
    el.setAttribute("data-t08probe", "1");
    return true;
  })()`);
}

async function clickByText(selector, text) {
  const found = await markByText(selector, text);
  if (!found) {
    throw new Error(`找不到包含「${text}」的 ${selector}`);
  }
  await clickSelector('[data-t08probe="1"]');
}

console.log(`目标应用：${APP_URL}`);

// --- 1. 未登录访问受保护深链接 → 跳登录页并保留 next ---------------------
await cdp.send("Page.navigate", { url: `${APP_URL}/settings` }, true);
await waitFor("跳转到登录页", `location.pathname === "/login"`);
const loginUrl = await evaluate("location.pathname + location.search");
check("未登录访问 /settings 跳转 /login 且保留 next", loginUrl === "/login?next=%2Fsettings", loginUrl);
check(
  "登录页显示会话过期提示",
  await evaluate(`document.body.innerText.includes("登录已过期")`),
);
await screenshot("01-redirect-login.png");

check(
  "密码框属性 type=password / autocomplete=current-password",
  await evaluate(`(() => {
    const input = document.getElementById("field-password");
    return input !== null && input.type === "password" && input.autocomplete === "current-password";
  })()`),
);

// --- 2. 键盘 Tab 走登录表单（含真实键盘提交）-----------------------------
await evaluate("document.body.focus()");
await pressTab();
const firstFocus = await evaluate("document.activeElement.id || document.activeElement.tagName");
check("Tab 第一次聚焦密码框", firstFocus === "field-password", String(firstFocus));
await screenshot("02-tab-focus-password.png");

// 用真实键盘输入错误密码；按钮此时才可用（空密码时按钮 disabled，Tab 会跳过它）。
await typeText("wrong-password");
await pressTab();
const secondFocus = await evaluate(
  "document.activeElement.tagName + ':' + (document.activeElement.textContent || '').trim()",
);
check("Tab 第二次聚焦登录按钮", secondFocus.startsWith("BUTTON:登录"), String(secondFocus));
await screenshot("03-tab-focus-login-button.png");
await pressEnter();
await waitFor("错误摘要出现", `document.querySelector('[role="alert"]') !== null`);
const alertText = await evaluate(`document.querySelector('[role="alert"]').innerText`);
check("错误密码显示「密码不正确」", alertText.includes("密码不正确"), alertText.replace(/\n/g, " "));
check(
  "焦点移到错误摘要",
  await evaluate(`document.activeElement.getAttribute("role") === "alert"`),
);
check(
  "字段与错误关联（aria-describedby）",
  await evaluate(`(() => {
    const input = document.getElementById("field-password");
    const described = input.getAttribute("aria-describedby");
    return described === "field-password-error" &&
      (document.getElementById("field-password-error")?.innerText ?? "").includes("密码不正确");
  })()`),
);
await screenshot("04-login-error.png");

// --- 4. 正确密码 → 回到 next 指定的设置页 → 资料库空态 --------------------
await clickSelector("#field-password");
await clearFocusedField();
const typed = await evaluate("document.getElementById('field-password').value");
check("清空后可重新输入（退格清空）", typed === "", JSON.stringify(typed));
await typeText(PASSWORD);
await clickByText("button", "登录");
await waitFor("登录后离开登录页", `location.pathname !== "/login"`, 20000);
const afterLogin = await evaluate("location.pathname");
check("登录后回到 next 指定的页面", afterLogin === "/settings", afterLogin);
await waitFor("设置页渲染服务状态", `document.body.innerText.includes("供应商配置")`, 20000);
check(
  "设置页显示 provider 配置状态、限制与健康检查（真实 /settings/status + health）",
  await evaluate(`(() => {
    const text = document.body.innerText;
    return text.includes("Tripo（模型生成）") && text.includes("说明书 AI") &&
      text.includes("生效的输入限制") && text.includes("健康检查") && text.includes("部署边界");
  })()`),
);
check(
  "设置页不含密钥输入框与 TLS/证书配置入口",
  await evaluate(`(() => {
    const text = document.body.innerText;
    return document.querySelector('input[type="password"]') === null &&
      text.includes("不显示也不接受任何密钥");
  })()`),
);
await screenshot("05-settings-page.png");

await cdp.send("Page.navigate", { url: `${APP_URL}/` }, true);
await waitFor("资料库空态", `document.body.innerText.includes("还没有物品")`, 20000);
check("登录成功进入资料库并显示空态", true);
check(
  "顶栏出现登出",
  await evaluate(`[...document.querySelectorAll("button")].some((b) => b.innerText.trim() === "登出")`),
);
await screenshot("06-library-empty.png");

// --- 5. 真实新建物品（真实 POST + CSRF 注入）----------------------------
await clickByText("a", "新建物品");
await waitFor("打开新建表单", `document.body.innerText.includes("新建物品") && !!document.getElementById("field-name")`);
await clickSelector("#field-name");
await typeText("T08 联调相机");
await clickSelector("#field-model");
await typeText("X100V-T08");
await clickSelector("#field-brand");
await typeText("Fujifilm");
await clickByText("button", "创建并继续");
await waitFor("跳转物品概览", `/^\\/items\\/[0-9a-f-]+$/.test(location.pathname)`, 20000);
const itemUrl = await evaluate("location.pathname");
const itemName = await evaluate(`document.querySelector("h1").innerText`);
check("创建走通真实 API 并跳转物品概览", itemName.includes("T08 联调相机"), `${itemUrl} / ${itemName}`);
check(
  "物品概览显示新物品型号与版本",
  await evaluate(`document.body.innerText.includes("X100V-T08") && /r\\d+/.test(document.body.innerText)`),
);
await screenshot("07-item-overview.png");

await cdp.send("Page.navigate", { url: `${APP_URL}/` }, true);
await waitFor("资料库出现新物品", `document.body.innerText.includes("T08 联调相机")`, 20000);
await screenshot("08-library-with-item.png");

// --- 6. 清除 cookie 后的会话过期处理（任意 API 401）---------------------
await cdp.send("Network.clearBrowserCookies", {}, true);
await clickByText("a", "设置");
await waitFor("会话过期回到登录页", `location.pathname === "/login"`, 20000);
const expiredUrl = await evaluate("location.pathname + location.search");
check("清 cookie 后请求 401 → 跳登录页并保留 next", expiredUrl === "/login?next=%2Fsettings", expiredUrl);
check(
  "登录页显示「登录已过期」提示",
  await evaluate(`document.body.innerText.includes("登录已过期")`),
);
await screenshot("09-session-expired.png");

// --- 7. 登出 -------------------------------------------------------------
await clickSelector("#field-password");
await typeText(PASSWORD);
await clickByText("button", "登录");
await waitFor("重新登录成功", `location.pathname !== "/login"`, 20000);
await cdp.send("Page.navigate", { url: `${APP_URL}/` }, true);
await waitFor("进入资料库", `document.body.innerText.includes("T08 联调相机")`, 20000);
check("重新登录后仍能看到之前创建的物品（数据持久化）", true);
await clickByText("button", "登出");
await waitFor("登出回到登录页", `location.pathname === "/login"`, 20000);
check("登出回到登录页", true);
const cookieAfterLogout = await evaluate("document.cookie.length === 0 ? '无 JS 可见 cookie' : document.cookie");
check("会话 cookie 为 HttpOnly（JS 不可见）", cookieAfterLogout === "无 JS 可见 cookie", cookieAfterLogout);

// --- 8. 窄屏抽屉（600px）-------------------------------------------------
await cdp.send("Emulation.setDeviceMetricsOverride", {
  width: 600,
  height: 900,
  deviceScaleFactor: 1,
  mobile: false,
});
await clickSelector("#field-password");
await typeText(PASSWORD);
await clickByText("button", "登录");
await waitFor("进入资料库", `location.pathname === "/"`, 20000);
await waitFor("窄屏出现抽屉触发按钮", `[...document.querySelectorAll("button")].some((b) => b.innerText.trim() === "资料库摘要")`);
check(
  "窄屏不并排显示侧栏（无 complementary 区域）",
  await evaluate(`document.querySelector('[role="complementary"]') === null`),
);
await clickByText("button", "资料库摘要");
await waitFor("抽屉打开", `document.querySelector('[role="dialog"]') !== null`);
const focusInDrawer = await evaluate(
  `document.querySelector('[role="dialog"]').contains(document.activeElement)`,
);
check("抽屉打开后焦点进入抽屉", focusInDrawer === true);
await screenshot("10-narrow-drawer.png");
await pressEscape();
await waitFor("抽屉关闭", `document.querySelector('[role="dialog"]') === null`);
check(
  "Esc 关闭抽屉并把焦点归还触发按钮",
  await evaluate(`(() => {
    const active = document.activeElement;
    return active !== null && active.tagName === "BUTTON" && active.innerText.trim() === "资料库摘要";
  })()`),
);

// --- 9. prefers-reduced-motion: reduce ----------------------------------
await cdp.send("Emulation.setDeviceMetricsOverride", {
  width: 1400,
  height: 900,
  deviceScaleFactor: 1,
  mobile: false,
});
await cdp.send(
  "Emulation.setEmulatedMedia",
  { features: [{ name: "prefers-reduced-motion", value: "reduce" }] },
  true,
);
const probeAnimationSeconds = () => evaluate(`(() => {
  const probe = document.createElement("div");
  probe.className = "skeleton__bar";
  document.body.appendChild(probe);
  const seconds = parseFloat(getComputedStyle(probe).animationDuration);
  probe.remove();
  return seconds;
})()`);
const reducedSeconds = await probeAnimationSeconds();
check(
  "prefers-reduced-motion 下骨架动画被关闭",
  reducedSeconds < 0.001,
  `animation-duration=${reducedSeconds}s`,
);
await cdp.send("Emulation.setEmulatedMedia", { features: [] }, true);
const normalSeconds = await probeAnimationSeconds();
check(
  "未开启减少动效时骨架动画仍存在（对照）",
  normalSeconds > 0.1,
  `animation-duration=${normalSeconds}s`,
);

// --- 10. 页面无未捕获异常 / console error --------------------------------
check(
  "浏览器无未捕获异常",
  cdp.exceptions.length === 0,
  cdp.exceptions.slice(0, 3).join(" | "),
);
check(
  "无 console.error（React/网络错误）",
  cdp.consoleErrors.length === 0,
  cdp.consoleErrors.slice(0, 3).join(" | "),
);

console.log("");
const passed = checks.filter((entry) => entry.ok).length;
console.log(`浏览器联调结果：${passed}/${checks.length} 项通过`);
if (failed > 0) {
  console.log("失败项：");
  for (const entry of checks.filter((e) => !e.ok)) {
    console.log(`  - ${entry.name} ${entry.detail}`);
  }
}
await cdp.send("Target.closeTarget", { targetId: page.targetId }, false);
process.exit(failed === 0 ? 0 : 1);
