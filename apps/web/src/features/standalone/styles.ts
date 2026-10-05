/**
 * 离线 3D 阅读器的内联样式（新导出 HTML 使用；独立文件可在无服务端/断网时显示）。
 *
 * VS-03 起取值与应用内阅读器共用 `design/vs-01/tokens.css` 的同一套设计语义
 * （纸面/墨色/工业橙、字号层级、2px 面板 / 4px 控件、3px 焦点环、140ms 过渡）；
 * 离线文件无法引用 theme.css，这里以 token 取值内联，变量命名与 tokens 对齐。
 * 约束：不引入网络字体、图片或 CDN；`--duration-*` 在减少动效偏好下归零。
 */
export const STANDALONE_STYLES = `
:root{color-scheme:light;
  --color-paper:#F4F1E8;--color-surface:#FFFEF9;--color-ink:#232620;--color-ink-muted:#62665D;--color-ink-on-accent:#FFFFFF;
  --color-accent:#A63F21;--color-accent-hover:#8F3518;--color-accent-active:#7A2C12;--color-selected:#EEE2CF;
  --color-success:#2F6248;--color-warning:#805D12;--color-danger:#A12F2F;--color-surface-soft:#EDEAE0;--color-surface-hover:#F0EDE2;
  --color-line:#D5D1C5;--color-line-control:#7D8176;--color-grid-line:rgba(35,38,32,.06);
  --radius-panel:2px;--radius-control:4px;--radius-plate:2px;
  --font-sans:-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif;
  --font-mono:ui-monospace,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace;
  --font-size-page-title:28px;--line-height-page-title:36px;--font-size-body:14px;--line-height-body:22px;
  --duration-control:140ms;
  font-family:var(--font-sans);font-size:var(--font-size-body);line-height:var(--line-height-body);-webkit-font-smoothing:antialiased}
@media (max-width:767px){:root{--font-size-page-title:24px;--line-height-page-title:32px}}
@media (prefers-reduced-motion:reduce){:root{--duration-control:0ms}}
*{box-sizing:border-box}
body{margin:0;min-width:320px;background:var(--color-paper);color:var(--color-ink)}
h1{margin:0;font-size:var(--font-size-page-title);line-height:var(--line-height-page-title);font-weight:600}
h2{margin:0 0 12px;font-size:18px;line-height:26px;font-weight:600}
h3{margin:0 0 8px;font-size:14px;font-weight:600}
p{margin:0 0 8px;overflow-wrap:anywhere}
a{color:var(--color-accent)}
.mono{font-family:var(--font-mono)}
button{font:inherit;min-height:44px;min-width:44px;padding:7px 13px;font-size:14px;font-weight:500;color:var(--color-ink);background:var(--color-surface);border:1px solid var(--color-line-control);border-radius:var(--radius-control);cursor:pointer;transition:background var(--duration-control) ease,border-color var(--duration-control) ease}
button:hover:not(:disabled){background:var(--color-surface-hover)}
button:disabled{opacity:1;background:var(--color-surface-soft);border-color:var(--color-line);color:var(--color-ink-muted);cursor:not-allowed}
:focus-visible{outline:3px solid var(--color-accent);outline-offset:3px}
.top{display:flex;justify-content:space-between;align-items:flex-end;gap:16px;padding:24px 28px 16px;border-bottom:1px solid var(--color-line)}
.eyebrow{margin:0 0 4px;font-family:var(--font-mono);font-size:12px;line-height:18px;letter-spacing:.14em;text-transform:uppercase;color:var(--color-ink-muted)}
.subtitle,.meta{color:var(--color-ink-muted);margin:0}
.meta{font-size:12px;line-height:18px}
.version-details{margin-top:6px;font-size:12px;line-height:18px;color:var(--color-ink-muted)}
.version-details summary{cursor:pointer;min-height:44px;display:flex;align-items:center}
.version-details p{margin:0;overflow-wrap:anywhere}
.layout{display:grid;grid-template-columns:260px minmax(0,1fr) 320px;gap:16px;padding:16px 28px 28px;height:calc(100vh - 128px);min-height:520px}
.panel{background:var(--color-surface);border:1px solid var(--color-line);border-radius:var(--radius-panel);padding:18px;overflow:auto}
.stage{position:relative;display:flex;flex-direction:column;background:var(--color-surface);border:1px solid var(--color-line);border-radius:var(--radius-panel);overflow:hidden;padding:8px;gap:8px}
.stage__bar{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.stage__bar button{min-height:32px;padding:0 10px;font-size:12px;background:var(--color-surface)}
.stage__canvas{flex:1;min-height:320px;border:1px solid var(--color-line);border-radius:var(--radius-plate);background-color:var(--color-paper);background-image:linear-gradient(var(--color-grid-line) 1px,transparent 1px),linear-gradient(90deg,var(--color-grid-line) 1px,transparent 1px);background-size:24px 24px;overflow:hidden}
.stage__canvas canvas{display:block;width:100%!important;height:100%!important;touch-action:none}
.status{margin:0;padding:0 4px;font-size:12px;line-height:18px;color:var(--color-ink-muted);flex-shrink:0}
.status.is-error{color:var(--color-danger)}
.parts{list-style:none;margin:0;padding:0}
.part{padding:12px 0;border-bottom:1px solid var(--color-line)}
.part:last-child{border-bottom:0}
.part__name{display:block;width:100%;text-align:left;font-weight:500}
.part.is-selected .part__name{background:var(--color-selected);border-color:var(--color-accent);color:var(--color-ink)}
.part p{color:var(--color-ink-muted);margin:6px 0 0;font-size:12px;line-height:18px}
.tag{display:inline-flex;align-items:center;margin:6px 0 0;padding:1px 8px;font-size:12px;line-height:18px;border:1px solid var(--color-line);border-radius:var(--radius-plate);background:var(--color-surface);color:var(--color-ink)}
.source{font-size:12px;line-height:18px}
.stepper{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:12px;font-size:12px;line-height:18px;color:var(--color-ink-muted)}
.stepper button{min-height:32px;padding:0 10px;font-size:12px}
.step ol{margin:0 0 10px;padding-left:20px}
.step li{margin-bottom:6px}
.warning{color:var(--color-danger)}
#em-specs-section{margin-top:20px;padding-top:16px;border-top:1px solid var(--color-line)}
.specs{display:grid;grid-template-columns:auto 1fr;gap:6px 12px;margin:0}
.specs dt{color:var(--color-ink-muted)}
.specs dd{margin:0;font-weight:500}
.interactions{display:grid;gap:8px;padding:10px 12px;border:1px solid var(--color-line);border-radius:var(--radius-panel);background:var(--color-surface);max-height:48vh;overflow:auto;flex:0 1 auto;min-height:0}
.interactions[hidden]{display:none}
.chips{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.chips__label{font-size:12px;line-height:18px;color:var(--color-ink-muted);min-width:2.5em}
.mesh-explorer{margin-bottom:16px;padding-bottom:16px;border-bottom:1px solid var(--color-line)}
.mesh-explorer h3{margin:0 0 8px;font-size:14px}
.mesh-explorer input{width:100%;min-height:40px;padding:9px 12px;border:1px solid var(--color-line-control);border-radius:var(--radius-control);background:var(--color-surface);color:var(--color-ink);font:inherit}
.mesh-explorer__list{display:grid;grid-template-columns:repeat(auto-fill,minmax(95px,1fr));gap:6px;max-height:160px;overflow:auto;margin:10px 0;padding:2px}
.mesh-explorer__list button{width:100%;min-height:40px;font-size:12px}
.mesh-explorer__list button[hidden]{display:none}
.chip{min-height:32px;padding:0 10px;border-radius:var(--radius-control);font-size:12px}
.chip[aria-pressed=true],.chip[aria-pressed=true]:hover:not(:disabled){background:var(--color-accent);border-color:var(--color-accent);color:var(--color-ink-on-accent)}
@media (max-width:900px){.layout{grid-template-columns:1fr;height:auto}.stage{min-height:60vh}.stage__canvas{height:55vh;flex:none}.interactions{max-height:none}.top{flex-direction:column;align-items:flex-start}.stage__bar button,.stepper button,.chip,.mesh-explorer__list button{min-height:44px;min-width:44px;font-size:14px}}
`;
