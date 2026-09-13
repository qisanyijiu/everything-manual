#!/usr/bin/env node
/** 诊断脚本：验证「Page.reload 后 Runtime.evaluate 是否卡住」是否为 CDP 客户端/浏览器行为，而非被测应用问题。 */
const CDP_PORT = process.env.CDP_PORT ?? "9555";
const info = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`).then((r) => r.json());
const ws = new WebSocket(info.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener("open", resolve);
  ws.addEventListener("error", reject);
});
let nextId = 1;
const pending = new Map();
ws.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.id === undefined) return;
  const entry = pending.get(message.id);
  if (entry === undefined) return;
  pending.delete(message.id);
  clearTimeout(entry.timer);
  if (message.error) entry.reject(new Error(message.error.message));
  else entry.resolve(message.result);
});
function send(method, params = {}, sessionId) {
  const id = nextId++;
  const payload = { id, method, params };
  if (sessionId) payload.sessionId = sessionId;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`超时：${method}`)), 10000);
    pending.set(id, { resolve, reject, timer });
    ws.send(JSON.stringify(payload));
  });
}
const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId: S } = await send("Target.attachToTarget", { targetId, flatten: true });
await send("Page.enable", {}, S);
await send("Runtime.enable", {}, S);
await send("Network.enable", {}, S);

async function probe(label) {
  try {
    const result = await send("Runtime.evaluate", { expression: "1+1", returnByValue: true }, S);
    console.log(`[PASS] ${label} → ${result.result.value}`);
  } catch (error) {
    console.log(`[FAIL] ${label} → ${error.message}`);
  }
}

const html = "data:text/html,<title>probe</title><p id=x>hello</p>";
await send("Page.navigate", { url: html }, S);
await new Promise((r) => setTimeout(r, 500));
await probe("navigate 后 evaluate");
await send("Page.reload", {}, S);
await new Promise((r) => setTimeout(r, 1500));
await probe("Page.reload 后 evaluate（1.5s）");
await send("Page.reload", {}, S);
await new Promise((r) => setTimeout(r, 300));
await probe("Page.reload 后 evaluate（0.3s，导航中）");
await new Promise((r) => setTimeout(r, 2000));
await probe("再等 2s 后 evaluate");
console.log("probe 结束");
process.exit(0);
