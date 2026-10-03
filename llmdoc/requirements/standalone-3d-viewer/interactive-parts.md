# 分件模型、自动热点绑定与交互层（PENTAX 17 / CyberDog 2）

状态：已实现并在真实 Provider 上跑通（分支 `feat/standalone-3d-viewer`），未经独立 QA。日期：2026-10-03。决策见 [ADR-042](../../decisions.md)。

## Tripo API 能力调研（developers.tripo3d.ai/llms.txt）

| 能力 | 结论 |
| --- | --- |
| `POST /mesh/segment` v2（语义分割） | 采用：输出多节点 GLB，节点只含平移，与原模型同坐标系；40 credits；`detailed` 粒度下 CyberDog 34 个部件、四条腿大腿/小腿基本分开（左后腿一体）。 |
| `generate_parts` | 未用：与 `texture` / `pbr` 不兼容，会丢失外观。 |
| `rig-check` / `rig` / `retarget` | 未用：CyberDog `riggable:false`（rig_type=quadruped）；四足预设只有 walk。姿势改为分件 + 关节枢轴的程序化刚体变换。 |
| `mesh-complete` | 未用：分件的遮挡面补全，当前展示不需要。 |

## 自动绑定结果

| 说明书 | 视图 | 轮廓 IoU | 编号→命中 | LLM 编号→部件 |
| --- | --- | --- | --- | --- |
| PENTAX 17（42 页） | 第 6 页正面 / 背面线稿 | 0.956 / 0.967 | 16 + 10，全部落在正确分件 | 32/32 |
| CyberDog 2（23 页） | 第 2 页两张线稿 | 0.859 / 0.711 | 7 + 10（1 个圆圈在轮廓外，吸附到最近可见面） | 12/17（头部、扩展接口槽等 5 项说明书条目里没有对应部件） |

投影核对图：[pentax-front](../../../artifacts/standalone-3d-viewer/autobind/pentax-front-projection.png)、[pentax-back](../../../artifacts/standalone-3d-viewer/autobind/pentax-back-projection.png)、[cyberdog-top](../../../artifacts/standalone-3d-viewer/autobind/cyberdog-top-projection.png)、[cyberdog-bottom](../../../artifacts/standalone-3d-viewer/autobind/cyberdog-bottom-projection.png)。

## 交互

- PENTAX 17：取下手柄／电池盖（toggle）、扳动卷片杆、按下快门、转动模式转盘（pulse）、打开后盖（toggle）。截图：[pentax-grip-open](../../../artifacts/standalone-3d-viewer/autobind/pentax-grip-open.png)、[reader](../../../artifacts/standalone-3d-viewer/pentax-reader-grip-open.png)。
- CyberDog 2：姿势 站立 / 趴下 / 坐下 / 握手 / 作揖（先膝后髋、自动贴地），动作 点头 / 摇头。截图：[poses](../../../artifacts/standalone-3d-viewer/autobind/cyberdog-poses.jpg)、[reader](../../../artifacts/standalone-3d-viewer/cyberdog-reader-sit.png)、[离线](../../../artifacts/standalone-3d-viewer/offline-cyberdog-lie.png)。
- 热点标记随所属部件一起运动；选中部件时整块分件高亮；相关动作排在前面。

## 验证

- Rust 全量通过（合并上游后 589 项；含交互层校验 4 项、迁移 v9 断言更新）；前端合并后 287 项通过（含动画/读取 6 项），typecheck / lint / build 通过。
- 浏览器：结果页、阅读页、离线 HTML（拦截全部网络、0 次请求、0 错误）上动作与姿势均可触发。
- 两个物品都已发布：release 资产为 `model` + `model_parts` + `document`，导出 ZIP 含分件 GLB。

## 已知限制

- 姿势是外观示意，关节枢轴取自分件包围盒，个别姿态（坐下）大腿与机身有穿插；左后腿分件为一体，只能整腿转动。
- 发布演示中由脚本代替人工确认候选热点与条目；真实使用须在复核页逐个核对。
- 自动绑定工具是原型脚本（`scripts/autobind/`），尚未接入服务端任务流水线。
