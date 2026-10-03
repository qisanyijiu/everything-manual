/** Loopback-only gateway: actual Rust Tripo polling sees scripted waiting/429 responses. */
import http from "node:http";
import { LocalFixture } from "./job-recovery-harness";
export type PollReply = { kind: "waiting" } | { kind: "retry"; after: number };
export class Pc05cProviderFixture {
  private readonly delegate = new LocalFixture();
  private server: http.Server | null = null;
  private port = 0;
  readonly taskReplies: PollReply[] = [];
  readonly observed: Array<{ at: number; kind: string; after?: number }> = [];
  get state() { return this.delegate.state; }
  get base() { return `http://127.0.0.1:${this.port}`; }
  get counts() { return { ...this.delegate.counts, task: this.observed.length }; }
  async start() {
    await this.delegate.start();
    this.server = http.createServer((req, res) => { void this.handle(req, res); });
    await new Promise<void>(resolve => this.server!.listen(0, "127.0.0.1", resolve));
    const addr = this.server.address(); if (!addr || typeof addr === "string") throw new Error("No owned fixture port"); this.port = addr.port;
  }
  private async handle(req: http.IncomingMessage, res: http.ServerResponse) {
    try {
      const pathname = new URL(req.url ?? "/", this.base).pathname;
      if (req.method === "GET" && pathname.startsWith("/v3/tasks/")) {
        const reply = this.taskReplies.shift(); this.observed.push({ at: Date.now(), kind: reply?.kind ?? "success", ...(reply?.kind === "retry" ? { after: reply.after } : {}) });
        if (reply) {
          if (reply.kind === "retry") { res.writeHead(429, { "content-type": "application/json", "retry-after": String(reply.after) }); res.end(JSON.stringify({ code: 429, message: "PC05C loopback retry fixture" })); }
          else { res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify({ code: 0, data: { task_id: "t17-fixture-task-0001", status: "running", progress: 1 } })); }
          return;
        }
      }
      const chunks: Buffer[] = []; for await (const chunk of req) chunks.push(Buffer.from(chunk));
      const body = Buffer.concat(chunks); const headers = new Headers();
      for (const [name, value] of Object.entries(req.headers)) if (value !== undefined && !["host", "content-length", "connection"].includes(name)) headers.set(name, Array.isArray(value) ? value.join(",") : value);
      const upstream = await fetch(this.delegate.base + (req.url ?? "/"), { method: req.method, headers, body: body.length ? body : undefined, redirect: "error" });
      res.writeHead(upstream.status, Object.fromEntries(upstream.headers)); res.end(Buffer.from(await upstream.arrayBuffer()));
    } catch { if (!res.headersSent) res.writeHead(500, { "content-type": "application/json" }); res.end('{"error":"owned fixture failure"}'); }
  }
  async stop() {
    if (this.server) { this.server.closeAllConnections(); await new Promise<void>(resolve => this.server!.close(() => resolve())); this.server = null; }
    await this.delegate.stop();
  }
}
