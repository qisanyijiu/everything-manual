import { useEffect, useId, useRef, useState } from "react";

import { fetchReleaseArchive, releaseDownloadError } from "../../api/release-download";

export const RELEASE_DOWNLOAD_DESCRIPTION =
  "包含原件、模型、manifest 与哈希，用于该发布版本的数据便携与灾备。不是整库备份，也不承诺双击运行网站。";

type DownloadState =
  | { status: "idle" | "pending" }
  | { status: "success"; filename: string }
  | { status: "error"; message: string; requestId: string | null };

interface Props {
  readonly releaseId: string;
  readonly descriptionId: string;
  /** Extra version identity for the list's otherwise identical buttons. */
  readonly versionLabel?: string;
}

/** One mounted instance per immutable release; never survives a route/version change. */
export function ReleaseDownload({ releaseId, descriptionId, versionLabel }: Props) {
  const statusId = useId();
  const [state, setState] = useState<DownloadState>({ status: "idle" });
  const active = useRef<AbortController | null>(null);
  const resources = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  useEffect(() => {
    const urls = resources.current;
    return () => {
      active.current?.abort();
      active.current = null;
      for (const [url, timer] of urls) {
        clearTimeout(timer);
        URL.revokeObjectURL(url);
      }
      urls.clear();
    };
  }, [releaseId]);

  const download = async () => {
    if (active.current !== null || releaseId === "") return;
    const controller = new AbortController();
    active.current = controller;
    setState({ status: "pending" });
    try {
      const { blob, filename } = await fetchReleaseArchive(releaseId, controller.signal);
      if (controller.signal.aborted) return;
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = filename;
      // Revoke after the browser has consumed the click; unmount also releases it.
      resources.current.set(url, setTimeout(() => {
        URL.revokeObjectURL(url);
        resources.current.delete(url);
      }, 1_000));
      anchor.hidden = true;
      document.body.append(anchor);
      try { anchor.click(); } finally { anchor.remove(); }
      setState({ status: "success", filename });
    } catch (error) {
      if (!controller.signal.aborted) setState({ status: "error", ...releaseDownloadError(error) });
    } finally {
      if (active.current === controller) active.current = null;
    }
  };

  const label = state.status === "pending" ? "正在打包…"
    : state.status === "error" ? "重试下载" : "下载说明书资料包";

  return (
    <div className="release-download" data-testid={`release-download-${releaseId}`}>
      <button
        type="button"
        className="button release-download__button"
        aria-label={versionLabel === undefined ? label : `${label}，${versionLabel}`}
        aria-describedby={`${descriptionId} ${statusId}`}
        aria-disabled={state.status === "pending"}
        onClick={() => { void download(); }}
      >{label}</button>
      {state.status === "error" ? (
        <p id={statusId} className="release-download__feedback release-download__feedback--error" role="alert">
          {state.message}
          {state.requestId !== null && <span>诊断 ID：{state.requestId}</span>}
        </p>
      ) : (
        <p id={statusId} className="release-download__feedback" role="status">
          {state.status === "pending" && "正在准备此发布版本的资料包，可离开页面，返回后重新下载。"}
          {state.status === "success" && `已发起下载：${state.filename}。请在浏览器下载列表中查看。`}
        </p>
      )}
    </div>
  );
}
