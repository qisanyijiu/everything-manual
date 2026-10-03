# 自动热点绑定与交互定义（原型工具，ADR-042）

把说明书标注图上的编号自动绑定到 3D 模型表面，并生成分件交互（动作 / 姿势）。产物以**候选**状态写回草稿，必须在复核页人工确认后才能发布。

## 流程

1. `tripo.py`：对草稿模型的 Tripo 任务调用 `POST /mesh/segment`（v2），得到与模型同一坐标系的分件 GLB；用 `POST /items/{id}/drafts/{draftId}/parts-model` 挂载（服务端核对包围盒一致）。
2. PDF 解析：用 pypdfium2 渲染标注图页；矢量线稿可删除文字/引线对象得到干净视图。
3. `vision.py`：视觉模型（Responses + JSON Schema）在带坐标网格的图上定位每个编号的引线端点 / 圆圈中心。
4. `align.py`：搜索 yaw/pitch/roll，使模型轮廓与线稿轮廓 IoU 最大；z-buffer 拾取把 2D 点投到模型表面，得到 asset-root 局部坐标与所属分件节点（未命中时吸附到最近可见面）。
5. `llm.py`：LLM 把图例编号 → 草稿部件条目 id。
6. `autobind.py`：PATCH `hotspots`（candidate）+ `interactive.bindings/actions/poses`；`cdposes.py` 生成四足机器人姿势（先膝后髋、自动贴地）。
7. `shade.py` / `posepreview.py`：离线渲染分件着色图与姿势预览，用于人工核对。

## 环境变量

- `EM_LLM_API_KEY`：Responses 兼容网关密钥；`TRIPO_API_KEY`：Tripo Open API 密钥（`tsk_…`）；`HTTPS_PROXY`：访问 Tripo 海外站时的代理（可选）。

依赖（仅工具使用，不进生产）：`pypdfium2==4.30.0 pillow==11.0.0 numpy==2.0.2 trimesh==4.5.3`。
