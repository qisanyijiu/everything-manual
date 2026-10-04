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

## 一键运行

```sh
EM_PASSWORD=… EM_LLM_API_KEY=… TRIPO_API_KEY=… HTTPS_PROXY=… \
  python run_manual.py <pentax|cyberdog> [--publish] [--resume]
```

`run_manual.py` 依次执行：上传说明书与视图 → 报价/确认/生成（自动对账超时批次）→ Tripo 分件并挂载 →
图例读取（文字层优先，图片图例自动改用视觉模型）→ 视角拟合与投影 → LLM 编号↔部件 →
`interactions.py` 按绑定结果自动推导动作与姿势（不手工指定节点）→ 写回候选 →（`--publish`）确认并发布。
`--resume` 复用已生成的草稿与分件，只重做绑定与交互，不重复计费。

`interactions.py`：相机类按图例名称推导动作（电池盖/手柄 → 取下，后盖 → 打开，按钮 → 按下，转盘 → 转动，杆 → 扳动）；
四足机器人自动识别身体轴与头部朝向、四条腿（大腿/小腿/足垫），生成站立/趴下/坐下/握手/作揖与点头/摇头，并自动贴地。

## 环境变量

- `EM_LLM_API_KEY`：Responses 兼容网关密钥；`TRIPO_API_KEY`：Tripo Open API 密钥（`tsk_…`）；`HTTPS_PROXY`：访问 Tripo 海外站时的代理（可选）。

依赖（仅工具使用，不进生产）：`pypdfium2==4.30.0 pillow==11.0.0 numpy==2.0.2 trimesh==4.5.3`。
