/** QA-only native browser tools. All profiles and remote debug listeners are owned. */
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { chromium, firefox, type BrowserContext, type Page, type TestInfo } from "@playwright/test";

interface FirefoxTarget { consoleActor?: string; memoryActor?: string; }
interface Packet extends Record<string, unknown> {
  from?: string; type?: string; error?: string; message?: string; exception?: unknown;
  resultID?: string; result?: unknown;
  tabs?: { actor: string; url?: string }[];
  frame?: FirefoxTarget; process?: FirefoxTarget; target?: FirefoxTarget;
  processDescriptor?: { actor: string };
  jsObjectsSize?: number; jsStringsSize?: number; jsOtherSize?: number;
}
interface NativeChromiumSettings {
  settingsPrivate: { setDefaultZoom(factor: number, callback: () => void): void; getDefaultZoom(callback: (factor: number) => void): void };
  runtime?: { lastError?: { message: string } };
}
class FirefoxRdp {
  private buffer = Buffer.alloc(0);
  private messages: Packet[] = [];
  private waiters: { match: (packet: Packet) => boolean; resolve: (packet: Packet) => void; reject: (error: Error) => void; timer: NodeJS.Timeout }[] = [];
  constructor(private socket: net.Socket) {
    socket.on("data", (bytes) => {
      this.buffer = Buffer.concat([this.buffer, bytes]);
      for (;;) {
        const delimiter = this.buffer.indexOf(":");
        if (delimiter < 0) break;
        const length = Number(this.buffer.subarray(0, delimiter).toString());
        if (!Number.isSafeInteger(length) || length < 0) throw new Error("Invalid Firefox RDP frame");
        if (this.buffer.length < delimiter + 1 + length) break;
        const packet = JSON.parse(this.buffer.subarray(delimiter + 1, delimiter + 1 + length).toString()) as Packet;
        this.buffer = this.buffer.subarray(delimiter + 1 + length);
        const waiter = this.waiters.find((candidate) => candidate.match(packet));
        if (waiter) {
          this.waiters.splice(this.waiters.indexOf(waiter), 1); clearTimeout(waiter.timer); waiter.resolve(packet);
        } else this.messages.push(packet);
      }
    });
    socket.on("error", (error) => this.rejectPending(error));
    socket.on("close", () => this.rejectPending(new Error("Owned Firefox RDP connection closed")));
  }
  private rejectPending(error: Error) { for (const waiter of this.waiters.splice(0)) { clearTimeout(waiter.timer); waiter.reject(error); } }
  private wait(match: (packet: Packet) => boolean): Promise<Packet> {
    const index = this.messages.findIndex(match);
    if (index >= 0) return Promise.resolve(this.messages.splice(index, 1)[0]!);
    return new Promise((resolve, reject) => {
      const waiter = { match, resolve, reject, timer: setTimeout(() => { this.waiters.splice(this.waiters.indexOf(waiter), 1); reject(new Error("Firefox RDP reply timeout")); }, 10_000) };
      this.waiters.push(waiter);
    });
  }
  async request(to: string, type: string, parameters: Packet = {}): Promise<Packet> {
    const reply = this.wait((packet) => packet.from === to && !["tabListChanged", "processListChanged", "resources-available-array", "evaluationResult", "garbage-collection"].includes(packet.type as string));
    const json = JSON.stringify({ to, type, ...parameters }); this.socket.write(`${Buffer.byteLength(json)}:${json}`);
    const packet = await reply;
    if (packet.error) throw new Error(`Firefox RDP ${type}: ${String(packet.error)} ${String(packet.message ?? "")}`);
    return packet;
  }
  async evaluate(actor: string, text: string): Promise<unknown> {
    const result = await this.request(actor, "evaluateJSAsync", { text });
    const value = await this.wait((packet) => packet.from === actor && packet.type === "evaluationResult" && packet.resultID === result.resultID);
    if (value.exception) throw new Error(`Firefox native tool evaluation failed: ${JSON.stringify(value.exception)}`);
    return value.result;
  }
  async tabTarget(page: Page): Promise<FirefoxTarget> {
    const tabs = await this.request("root", "listTabs");
    const descriptor = tabs.tabs!.find((tab) => tab.url === page.url());
    if (!descriptor) throw new Error("Owned Firefox page target not found");
    return (await this.request(descriptor.actor, "getTarget")).frame!;
  }
  async chromeConsole(): Promise<string> {
    const descriptor = (await this.request("root", "getProcess", { id: 0 })).processDescriptor!;
    const packet = await this.request(descriptor.actor, "getTarget");
    const target = packet.process ?? packet.frame ?? packet.target;
    if (!target?.consoleActor) throw new Error("Firefox native browser console not available");
    return target.consoleActor;
  }
  close() { this.socket.destroy(); this.rejectPending(new Error("Owned Firefox RDP disposed")); }
  static async connect(port: number, version: string): Promise<FirefoxRdp> {
    if (!Number.isInteger(port) || port <= 0) throw new Error("An owned Firefox RDP loopback port is required");
    let socket: net.Socket | undefined;
    for (let count = 0; count < 30 && !socket; count += 1) {
      try { socket = await new Promise<net.Socket>((resolve, reject) => { const candidate = net.connect(port, "127.0.0.1", () => resolve(candidate)); candidate.once("error", reject); }); }
      catch { await new Promise((resolve) => setTimeout(resolve, 100)); }
    }
    if (!socket) throw new Error("Owned Firefox RDP listener unavailable");
    const client = new FirefoxRdp(socket);
    try { await client.wait((packet) => packet.from === "root"); await client.request("root", "connect", { frontendVersion: version }); return client; }
    catch (error) { client.close(); throw error; }
  }
}
export const FIREFOX_QA_PREFS = { "devtools.debugger.remote-enabled": true, "devtools.chrome.enabled": true, "devtools.debugger.prompt-connection": false, "devtools.debugger.force-local": true };
async function freePort(): Promise<number> {
  const server = net.createServer(); await new Promise<void>((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve));
  const address = server.address(); if (!address || typeof address === "string") throw new Error("No owned RDP port");
  await new Promise<void>((resolve) => server.close(() => resolve())); return address.port;
}
function withoutRdpArgs(args: string[]): string[] { const result: string[] = []; for (let index = 0; index < args.length; index += 1) { if (args[index] === "--start-debugger-server") index += 1; else result.push(args[index]!); } return result; }
export async function launchNativeZoom(testInfo: TestInfo): Promise<{
  context: BrowserContext; page: Page; engine: string; method: string;
  setZoom: (factor: number) => Promise<number>; nativeOuterWidth: () => Promise<number>; close: () => Promise<void>;
}> {
  const use = testInfo.project.use;
  const engine = use.browserName ?? "chromium";
  if (engine !== "chromium" && engine !== "firefox") throw new Error(`Native zoom QA has no ${engine} adapter`);
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "owned-native-browser-zoom-"));
  const launch = use.launchOptions ?? {};
  const debugPort = engine === "firefox" ? await freePort() : 0;
  let context: BrowserContext | undefined; let client: FirefoxRdp | undefined;
  try {
    context = await (engine === "firefox" ? firefox : chromium).launchPersistentContext(profile, {
      ...launch, ...(use.channel ? { channel: use.channel } : {}), headless: true,
      viewport: null, deviceScaleFactor: undefined, isMobile: undefined, locale: "zh-CN", baseURL: use.baseURL,
      args: [...withoutRdpArgs(launch.args ?? []), ...(engine === "firefox" ? ["--start-debugger-server", String(debugPort)] : []), "--window-size=1440,1000"],
      ...(engine === "firefox" ? { firefoxUserPrefs: { ...(launch.firefoxUserPrefs ?? {}), ...FIREFOX_QA_PREFS } } : {}),
    });
    const page = context.pages()[0] ?? await context.newPage();
    let setZoom: (factor: number) => Promise<number>; let nativeOuterWidth: () => Promise<number>;
    if (engine === "firefox") {
      client = await FirefoxRdp.connect(debugPort, context.browser()!.version()); const console = await client.chromeConsole();
      const findBrowser = () => `const uri=${JSON.stringify(page.url())};let found;for(const w of Services.wm.getEnumerator("navigator:browser")){for(const b of w.gBrowser.browsers){if(b.currentURI.spec===uri){found={w,b};break;}}if(found)break;}if(!found)throw new Error("Owned Firefox zoom tab missing");`;
      setZoom = async (factor) => Number(await client!.evaluate(console, `JSON.stringify((()=>{${findBrowser()}found.w.FullZoom.setZoom(${factor},found.b);return found.b.fullZoom;})())`));
      nativeOuterWidth = async () => Number(await client!.evaluate(console, `JSON.stringify((()=>{${findBrowser()}return found.w.outerWidth;})())`));
    } else {
      const settings = await context.newPage();
      const executable = String(launch.executablePath ?? "").toLowerCase();
      const edge = use.channel?.startsWith("msedge") || executable.includes("microsoft edge");
      await settings.goto(edge ? "edge://settings/appearance" : "chrome://settings/appearance");
      setZoom = async (factor) => {
        await settings.evaluate((zoom) => new Promise<void>((resolve, reject) => { const chrome = (globalThis as unknown as { chrome: NativeChromiumSettings }).chrome; chrome.settingsPrivate.setDefaultZoom(zoom, () => chrome.runtime?.lastError ? reject(new Error(chrome.runtime.lastError.message)) : resolve()); }), factor);
        const actual = await settings.evaluate(() => new Promise<number>((resolve) => (globalThis as unknown as { chrome: NativeChromiumSettings }).chrome.settingsPrivate.getDefaultZoom(resolve)));
        await page.bringToFront(); return actual;
      };
      nativeOuterWidth = () => page.evaluate(() => outerWidth);
    }
    return { context, page, engine, method: engine === "firefox" ? "Firefox native browser FullZoom.setZoom via owned RDP" : "Actual Chromium browser settingsPrivate.setDefaultZoom in owned profile", setZoom, nativeOuterWidth,
      close: async () => { client?.close(); try { await context!.close(); } finally { fs.rmSync(profile, { recursive: true, force: true }); } } };
  } catch (error) { client?.close(); try { await context?.close(); } finally { fs.rmSync(profile, { recursive: true, force: true }); } throw error; }
}
export interface FirefoxHeapSession {
  sample(options?: { gc?: boolean }): Promise<{ bytes: number; gcApplied: boolean }>;
  evidence(): { method: string; metricDefinition: string; measurements: Packet[] };
  detach(): Promise<void>;
}
export async function firefoxHeapSession(page: Page): Promise<FirefoxHeapSession> {
  const client = await FirefoxRdp.connect(Number(process.env.EM_QA_FIREFOX_RDP_PORT), page.context().browser()!.version());
  try {
    const target = await client.tabTarget(page); const actor = target.memoryActor;
    if (!actor) throw new Error("Firefox same-tab memory actor missing");
    await client.request(actor, "attach"); const measurements: Packet[] = [];
    return {
      sample: async (options) => {
        const gcApplied = options?.gc === true;
        if (gcApplied) await client.request(actor, "forceGarbageCollection");
        const raw = await client.request(actor, "measure");
        for (const key of ["jsObjectsSize", "jsStringsSize", "jsOtherSize"]) if (!Number.isFinite(raw[key]) || (raw[key] as number) < 0) throw new Error(`Firefox invalid ${key} measurement`);
        const bytes = raw.jsObjectsSize! + raw.jsStringsSize! + raw.jsOtherSize!;
        if (!(bytes > 0)) throw new Error("Firefox same-tab JS footprint must be positive");
        measurements.push({ ...raw, gcApplied, bytes }); return { bytes, gcApplied };
      },
      evidence: () => ({ method: "Firefox DevTools same-tab Memory actor measure/forceGarbageCollection", metricDefinition: "jsObjectsSize + jsStringsSize + jsOtherSize; Firefox target JS footprint; differs from Chromium JSHeapUsedSize; absolute bytes cannot be compared across engines", measurements }),
      detach: async () => { try { await client.request(actor, "detach"); } finally { client.close(); } },
    };
  } catch (error) { client.close(); throw error; }
}
