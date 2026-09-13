/**
 * 阅读/校准工作区（路由 `/items/:itemId/drafts/:draftId/review`）。
 *
 * T18 交付了 3D 阅读器与坐标层；T19 把本页升级为**校准工作区**：
 * 热点绑定/解绑/重新绑定、步骤视角、知识确认与人工修订、发布（带不变量明细）。
 * 实现（页面结构与面板）在 `features/manual/CalibrationWorkspace.tsx`——
 * 本文件只保留路由组件名与懒加载边界（App.tsx 的 `lazy()` 指向它）。
 */

export { CalibrationWorkspace as ReviewWorkspacePage } from "../manual/CalibrationWorkspace";
export { CalibrationWorkspace as default } from "../manual/CalibrationWorkspace";
