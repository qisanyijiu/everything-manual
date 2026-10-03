// Vitest 全局设置：注册 jest-dom 断言（使用 vitest 专用入口）。
import "@testing-library/jest-dom/vitest";

// jsdom has no top-layer dialog implementation. Browser/native focus and unload
// semantics are tested separately in a real browser, not inferred from this shim.
HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
