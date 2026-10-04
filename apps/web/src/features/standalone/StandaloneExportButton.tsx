/**
 * 「下载离线 3D 页面」按钮：把当前发布版本打成一个可双击打开的 HTML 文件。
 *
 * - 所有工作都在浏览器内完成：取物品信息与 GLB 字节 → 组装 HTML → 触发下载；
 * - 离线脚本（three.js + 阅读器）与组装逻辑只在点击时 lazy import，不进首屏包；
 * - 文件里没有会话、密钥或服务端地址，可以直接发给没有账号的读者。
 */

import { useState } from "react";

import { describeError } from "../../api/client";
import { fetchAssetContent, getItem } from "../../api/endpoints";
import { readDraftModel } from "../viewer/draft-view";

export interface StandaloneExportButtonProps {
  readonly itemId: string;
  readonly releaseId: string;
  readonly manifest: { readonly knowledge?: unknown } | null;
}

type ExportState =
  | { readonly phase: "idle" }
  | { readonly phase: "working"; readonly label: string }
  | { readonly phase: "done"; readonly fileName: string; readonly sizeMb: string }
  | { readonly phase: "error"; readonly message: string };

function triggerDownload(html: string, fileName: string): void {
  const url = URL.createObjectURL(new Blob([html], { type: "text/html;charset=utf-8" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

export function StandaloneExportButton({ itemId, releaseId, manifest }: StandaloneExportButtonProps) {
  const [state, setState] = useState<ExportState>({ phase: "idle" });
  const model = readDraftModel(manifest?.knowledge);

  const run = async (): Promise<void> => {
    if (model === null || manifest === null) {
      return;
    }
    try {
      setState({ phase: "working", label: "正在读取模型…" });
      const builder = await import("./build-html");
      const assetId = builder.standaloneModelAssetId(manifest) ?? model.assetId;
      const [item, bytes, viewer] = await Promise.all([
        getItem(itemId),
        fetchAssetContent(assetId),
        import("virtual:standalone-viewer"),
      ]);
      setState({ phase: "working", label: "正在生成离线页面…" });
      const info = { name: item.data.name, brand: item.data.brand ?? null, model: item.data.model ?? null };
      const payload = builder.buildStandalonePayload(info, { id: releaseId, manifest });
      const html = builder.buildStandaloneHtml(payload, new Uint8Array(bytes.bytes), viewer.default);
      const fileName = builder.standaloneFileName(info);
      triggerDownload(html, fileName);
      setState({ phase: "done", fileName, sizeMb: (html.length / 1024 / 1024).toFixed(1) });
    } catch (error) {
      setState({ phase: "error", message: describeError(error).message });
    }
  };

  return (
    <div className="standalone-export">
      <button
        type="button"
        className="button"
        onClick={() => void run()}
        disabled={model === null || state.phase === "working"}
        data-testid="standalone-export"
      >
        {state.phase === "working" ? state.label : "下载离线 3D 页面"}
      </button>
      <p className="field__hint" role="status">
        {model === null && "该版本没有可用模型，无法导出 3D 页面。"}
        {state.phase === "done" && `已生成 ${state.fileName}（约 ${state.sizeMb} MB）：双击即可在浏览器打开，无需登录或联网。`}
        {state.phase === "error" && `导出失败：${state.message}`}
      </p>
    </div>
  );
}
