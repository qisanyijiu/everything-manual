/**
 * 离线 3D 阅读器的内联样式（与 `theme.css` 同一套色板，独立文件可在无服务端时显示）。
 */
export const STANDALONE_STYLES = `
:root{color-scheme:light;--bg:#f7f8f4;--surface:#fffefa;--text:#243b33;--muted:#737b71;--border:#e2e6dd;--accent:#356552;--hot:#c8643c;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif;font-size:14px;line-height:1.65;-webkit-font-smoothing:antialiased}
*{box-sizing:border-box}
body{margin:0;min-width:320px;background:var(--bg);color:var(--text)}
h1{margin:0;font-size:26px;font-weight:650;letter-spacing:-.6px}
h2{margin:0 0 12px;font-size:16px;font-weight:650}
h3{margin:0 0 8px;font-size:14px}
p{margin:0 0 8px;overflow-wrap:anywhere}
button{font:inherit;min-height:36px;padding:7px 13px;font-size:13px;font-weight:550;color:var(--text);background:var(--surface);border:1px solid var(--border);border-radius:8px;cursor:pointer}
button:hover:not(:disabled){background:#f0f3ec;border-color:#b9c6b6}
button:disabled{opacity:.48;cursor:default}
:focus-visible{outline:3px solid #be693f;outline-offset:3px}
.top{display:flex;justify-content:space-between;align-items:flex-end;gap:16px;padding:24px 28px 16px}
.eyebrow{margin:0 0 4px;font-size:11px;letter-spacing:.22em;color:var(--muted)}
.subtitle,.meta{color:var(--muted);margin:0}
.meta{font-size:12px}
.layout{display:grid;grid-template-columns:260px minmax(0,1fr) 300px;gap:16px;padding:0 28px 28px;height:calc(100vh - 110px);min-height:520px}
.panel{background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:18px;overflow:auto}
.stage{position:relative;display:flex;flex-direction:column;background:#eef1ea;border:1px solid var(--border);border-radius:12px;overflow:hidden}
.stage__bar{position:absolute;top:12px;left:12px;z-index:1}
.stage__canvas{flex:1;min-height:320px}
.stage__canvas canvas{display:block;width:100%!important;height:100%!important;touch-action:none}
.status{margin:0;padding:10px 14px;font-size:12px;color:var(--accent);border-top:1px solid var(--border);background:var(--surface)}
.status.is-error{color:#b14837}
.parts{list-style:none;margin:0;padding:0}
.part{padding:12px 0;border-bottom:1px solid var(--border)}
.part:last-child{border-bottom:0}
.part.is-selected .part__name{background:var(--accent);border-color:var(--accent);color:#fff}
.part p{color:var(--muted);margin:6px 0 0}
.tag{display:inline-block;margin-left:8px;padding:2px 8px;font-size:12px;border-radius:999px;background:#e7efe3;color:var(--accent)}
.source{font-size:12px}
.stepper{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:12px;font-size:13px}
.step ol{margin:0 0 10px;padding-left:20px}
.step li{margin-bottom:6px}
.warning{color:#a95636}
#em-specs-section{margin-top:20px;padding-top:16px;border-top:1px solid var(--border)}
.specs{display:grid;grid-template-columns:auto 1fr;gap:6px 12px;margin:0}
.specs dt{color:var(--muted)}
.specs dd{margin:0;font-weight:550}
.interactions{display:grid;gap:8px;padding:10px 14px;border-top:1px solid var(--border);background:var(--surface)}
.interactions[hidden]{display:none}
.chips{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.chips__label{font-size:12px;color:var(--muted);min-width:2.5em}
.chip{min-height:32px;padding:5px 12px;border-radius:999px}
.chip[aria-pressed=true],.chip[aria-pressed=true]:hover:not(:disabled){background:var(--accent);border-color:var(--accent);color:#fff}
@media (max-width:900px){.layout{grid-template-columns:1fr;height:auto}.stage{height:60vh}.top{flex-direction:column;align-items:flex-start}}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
`;
