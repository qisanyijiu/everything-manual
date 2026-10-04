# 说明书 PDF 视图候选 + 拖拽排列（view-candidates）

状态：已实现（分支 `feat/standalone-3d-viewer`），未经独立 QA。日期：2026-10-03。决策见 [ADR-044](../../decisions.md)。

## 需求

PDF 解析时拆出说明书里可能是正面 / 左侧 / 背面 / 右侧 / 特写的图片作为待选项；用户在前端通过拖拽、删除确定视图的最终排列。

## 实现

| 层 | 内容 |
| --- | --- |
| 拆图（浏览器，PDF.js） | [`pdf/extract.ts`](../../../apps/web/src/features/import/pdf/extract.ts) + 纯函数 [`pdf/figures.ts`](../../../apps/web/src/features/import/pdf/figures.ts)：operator list 跟踪 CTM 得到内嵌位图框；不含文字渲染 → 墨迹掩膜（擦掉文字层）→ 膨胀 → 连通区域得到矢量线稿框；过滤图标/细长线/整页图，去重，每页 ≤4、总数 ≤24；高分辨率裁剪为 JPEG。 |
| 判断（服务端） | [`view_classify.rs`](../../../crates/server/src/providers/manual_ai/view_classify.rs)：同一说明书 AI 配置（密钥不出服务端），Responses + 严格 JSON Schema，输出 `isProductView / view / confidence / reason`。失败不阻塞。 |
| 存储 | 迁移 `0010_view_candidates`：候选行（资产、来源页、`embedded|region|upload`、建议视图、置信度、说明、软删除）。候选不进报价/生成快照。 |
| API | `GET/POST /items/{id}/view-candidates`、`POST …/{candidateId}/dismiss|restore`、`PUT /items/{id}/photos/arrangement`（一次性提交 5 个槽位；单事务替换照片行，互换视图不撞唯一索引；同资产保留照片 id）。 |
| 前端 | [`ViewArrangement.tsx`](../../../apps/web/src/features/import/ViewArrangement.tsx) + 纯逻辑 [`arrangement.ts`](../../../apps/web/src/features/import/arrangement.ts)：候选区 ↔ 5 槽位 HTML5 拖拽（放到已占槽 = 互换，拖回候选区 = 移出），每张卡片的下拉框是键盘/无鼠标替代；「删除」软删除并可撤销；「按建议填入空槽」；「保存排列」。不像产品视图的候选折叠在「其余 N 张」里。原逐槽上传保留在折叠区。 |

## 验证

- Rust：`tests/view_candidates.rs` 3 项（幂等/软删除/撤销、非照片资产拒绝、互换+清空+非法输入无副作用）；迁移断言更新到 v10。
- 前端：`figures.test.ts` 7 项、`arrangement.test.ts` 6 项；全量 300 项通过；typecheck / lint / build 通过。
- 真实浏览器 + 真实说明书 AI：CyberDog 2（23 页）拆出 24 张候选，12 张给出视图建议（正面 99%、背面 97%、特写 96% 等），12 张判为"不像产品视图"并折叠；自动填槽 + 拖拽 + 删除 + 保存后照片为 front/back/right/detail。截图：[cyberdog-arranged](../../../artifacts/view-candidates/cyberdog-arranged.png)。PENTAX 17 见 [pentax-arranged](../../../artifacts/view-candidates/pentax-arranged.png)。

## 已知限制

- 说明书 AI 对"多角度线稿"的方向判断偏向 front（三分之四视角常被判为正面），左/右/背面仍需用户拖拽修正——这正是拖拽排列存在的理由。
- 一张页面里若两幅图靠得很近，区域查找可能合成一个候选；可用逐槽上传替代。
- 拆图在浏览器内进行，40 页以内；超出部分不拆。
