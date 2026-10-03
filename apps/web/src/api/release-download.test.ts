import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, onUnauthorized } from "./client";
import { fetchReleaseArchive, releaseDownloadError, releaseDownloadFilename } from "./release-download";

function zipEnvelope(): Uint8Array<ArrayBuffer> {
  // A small ZIP envelope suffices for transport checks; real file entries/hashes
  // are independently exercised by release-download.spec.ts against Rust.
  const bytes = new Uint8Array(98);
  const data = new DataView(bytes.buffer);
  data.setUint32(0, 0x04034b50, true);
  data.setUint32(30, 0x02014b50, true);
  data.setUint32(76, 0x06054b50, true);
  data.setUint16(84, 1, true); data.setUint16(86, 1, true);
  data.setUint32(88, 46, true); data.setUint32(92, 30, true);
  return bytes;
}

afterEach(() => vi.unstubAllGlobals());

describe("published archive transport", () => {
  it("uses exactly the selected release, abort signal and session credentials", async () => {
    const fetch = vi.fn(async () => new Response(zipEnvelope(), {
      headers: { "content-type": "application/zip", "content-disposition": 'attachment; filename="book.zip"' },
    }));
    vi.stubGlobal("fetch", fetch);
    const controller = new AbortController();
    const archive = await fetchReleaseArchive("older-release", controller.signal);
    expect(fetch).toHaveBeenCalledWith("/api/v1/releases/older-release/export", expect.objectContaining({
      signal: controller.signal, credentials: "same-origin", cache: "no-store", method: "GET",
    }));
    expect(archive.filename).toBe("book.zip");
    expect(archive.blob.size).toBe(98);
  });

  it.each([
    ["application/json", '{"error":"not a zip"}'],
    ["text/html", "<html>/private/server-error</html>"],
    ["application/zip", '{"error":"disguised as ZIP"}'],
    ["application/zip", "PK\u0003\u0004truncated"],
  ])("rejects fake/truncated success: %s", async (mime, body) => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(body, { headers: { "content-type": mime, "x-request-id": "request-safe" } })));
    await expect(fetchReleaseArchive("release-a", new AbortController().signal)).rejects.toMatchObject({
      code: "INVALID_RELEASE_ARCHIVE", requestId: "request-safe",
    });
  });

  it("broadcasts 401 through the shared client without retrying", async () => {
    const listener = vi.fn();
    const unsubscribe = onUnauthorized(listener);
    const fetch = vi.fn(async () => new Response("<html>session expired</html>", { status: 401 }));
    vi.stubGlobal("fetch", fetch);
    await expect(fetchReleaseArchive("release-a", new AbortController().signal)).rejects.toMatchObject({ status: 401 });
    expect(listener).toHaveBeenCalledOnce(); expect(fetch).toHaveBeenCalledOnce();
    unsubscribe();
  });

  it.each([
    [null, "release-release-a.zip"],
    ['attachment; filename="manual.zip"', "manual.zip"],
    ["attachment; filename*=UTF-8''%E8%AF%B4%E6%98%8E%E4%B9%A6.zip", "说明书.zip"],
    ['attachment; filename="../../private.zip"', "release-release-a.zip"],
    ['attachment; filename="C:\\private.zip"', "release-release-a.zip"],
    ['attachment; filename="bad\u0000name.zip"', "release-release-a.zip"],
    ['attachment; filename="bad\u202ename.zip"', "release-release-a.zip"],
    ['attachment; filename="con.zip"', "release-release-a.zip"],
    ['attachment; filename="page.html"', "release-release-a.zip"],
    ["attachment; filename*=UTF-8''%QQ.zip", "release-release-a.zip"],
  ])("uses a safe filename (%s)", (header, expected) => {
    expect(releaseDownloadFilename(header, "release-a")).toBe(expected);
  });

  it.each([403, 404, 500])("does not expose HTTP %s response internals", (status) => {
    const info = releaseDownloadError(new ApiError(status, "INTERNAL", "<html>/private/server-error</html>", "request-safe"));
    expect(info.message).not.toMatch(/html|private|server-error/);
    expect(info.requestId).toBe("request-safe");
    expect(releaseDownloadError(new ApiError(status, "INTERNAL", "hidden", "/private/path")).requestId).toBeNull();
  });
});
