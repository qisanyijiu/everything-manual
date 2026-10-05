# Nikon F3HP：76 个编号分件观察

用户要求重新分件，让全部 76 个编号都能交互。本轮保留编号、重新调整几何分区边界，并给每件增加选中高亮、独立展开与复原，以及全部展开、全部复原和编号搜索。在线阅读器、草稿复核页和离线 HTML 使用同一套分件变换逻辑。

这些编号表示外观模型的几何区域，尚未全部对应说明书的真实部件。观察展开不表示真实拆卸方向，也不表示内部机械结构。已人工确认的取景器外壳与背带环绑定继续保留，其余编号不自动冒充说明书部件。

## 新模型与版本

| 内容 | 值 |
| --- | --- |
| 物品 | `01a10658-7951-7684-886d-2827a0ec2d8e` |
| 新发布版本 | `01a10799-3ea8-7200-a426-932c55c29235`，冻结草稿 r5；当前草稿 r6 |
| 新分件资产 | `01a10790-445a-73eb-8797-f1babaddbf5d` |
| 新分件 GLB SHA-256 | `ae818102425ccd9670c4663fe3c90a0437c68cdaaa8354fbacf0f34e339ec610` |
| 新 manifest SHA-256 | `11a6d9514a2ce9877e2225a34c66c943af8504aaabce0ff29950ef0034d031ad` |
| 几何数量 | 76 个非空、连通区域；94,129 个原三角面完整覆盖一次 |
| 原语义绑定 | 2 条绑定，节点 0/6/7 保持原面归属；保留 1 项取景器外壳示意动作 |
| 本轮供应商调用与费用 | 0 次，0 Tripo credits；此前真实生成的 70 credits 未变化 |

用本地曲率加权边界优化重新划分 73 个未锁定区域。1,008 个原三角面改变区域归属，加权边界代价降低 15.94%；保持节点 0/6/7 的几何归属，以延续已确认绑定。新 GLB 保留所有原三角面、绕序、法线、UV、材质和三张原贴图；世界坐标最大误差为 `1.83e-8`，整体包围盒误差为 `3.73e-9`。重复 dry-run 得到相同输出 SHA。没有用修改 pivot 或编号代替重分件，也没有调用不能保证固定数量及编号的供应商分割接口。

可复现工具为 [`scripts/resegment-parts.py`](../../../scripts/resegment-parts.py)，仅依赖 Python 标准库。运行 `python3 scripts/resegment-parts.py --help` 查看参数；默认 dry-run，写模型必须明确指定新输出路径，拒绝覆盖输入。工具不读 API key，也不操作数据库。本次通过带 `If-Match` 的现有上传 API 挂载新附件，再显式保存新不可变版本；原发布和原资产仍可阅读。

## 交互与验证

- 编号 `tripo_part_0` 至 `tripo_part_75` 均可从列表或网格点击选择，选择后高亮并可独立展开/复原。
- 全部展开与全部复原覆盖所有 76 件；观察层与说明书动作分开，复原观察层不会清掉正在显示的取景器示意动作。
- 热点随所属节点移动；保留旧版 payload 兼容，新的离线导出明确携带全部节点名。
- 全屏观察使用原有渲染器与状态，让模型和分件面板并排显示；退出按钮或 Esc 可返回原页面。

Chrome 实际验证在草稿页、新发布页和断网 `file://` 离线页分别执行 76 次独立展开/复原。每次检查选中状态、真实节点矩阵发生位移、另外 75 件矩阵保持不变，以及全部复原误差低于 `1e-10`。三处均验证全部展开、全部复原、编号搜索和与原取景器动作叠加后复原。没有只凭按钮数量或点击结果宣称通过。

离线页另验证 1280×800、1280×720 和 390×844 的可见布局，交互区与状态栏没有被舞台裁切；取景同时考虑水平与垂直视锥。浏览器无运行错误，无供应商或远程请求。仅测试 Google Chrome。

真实 Nikon 全屏页另一次逐件矩阵验收全部通过。1440×1000 中画布为 1052×856，右侧分件面板为 320×968，两者均在视口内；始终只有原来的一个 Canvas。全部展开后使用退出按钮，76 件状态保持；重新进入后全部复原，再通过实际 Esc 键退出。页面仅对本模型主栏的全屏处理 Esc，不退出其他元素的全屏。

最终 Linux 构建中全部 50 个前端测试文件、336 项测试通过。分件与全屏相关 22 项定向检查、TypeScript、ESLint 和生产构建通过。此次工作开始于 2026-10-04，最终全屏与 Linux 复核于 2026-10-05 完成。此前 Docker 交付记录保留历史事实，不用旧镜像的结果冒充此次构建结果。

原 2 个发布版本的清单 SHA 保持不变；原 361 个 blob 保留。更新后统一资料库为 8 个物品、7 份说明书/166 页、370 条资产、363 个 blob、3 份草稿及 3 个发布版本。SQLite 完整性与外键检查通过。供应商尝试仍为 42 条，费用账本仍为 10 条。冻结本机后端二进制及密文覆盖文件的 SHA 均未变化。

## 预览与证据

当前本机继续使用已经获准访问钥匙串的固定后端，并用统一脚本启动当前前端源码；不替换已授权程序，不导出主密钥或 API key。已保存源码前端偏好，后续仍只需：

```sh
bash scripts/start-project.sh
```

本机打开 `http://127.0.0.1:5173/`，后端为 `http://127.0.0.1:8080/`。显式启用为 `--source-ui`，回到程序内嵌页面为 `--embedded-ui`，`--status` 查看两者，`--stop` 停止两者。Docker 使用内嵌页面。全新克隆默认使用发行程序的内嵌页面，源码预览需要已安装前端依赖。

新版本预览：<http://127.0.0.1:5173/items/01a10658-7951-7684-886d-2827a0ec2d8e/releases/01a10799-3ea8-7200-a426-932c55c29235>。

私有验证产物都在被 Git 忽略的 `var/preview/validation/nikon-parts-20261004/`：

| 文件 | 内容 |
| --- | --- |
| `resegmentation/report.json` | 新几何分区、全部面覆盖、属性/贴图保留与重复性证据 |
| `resegmentation/nikon-f3hp-resegmented.glb` | 新 76 节点 GLB |
| `draft-e2e.json` / `release-e2e.json` / `offline-e2e.json` | 三处 Chrome 每件矩阵验证明细 |
| `offline-layouts.json` | 三种 Chrome 窗口尺寸布局检查 |
| `fullscreen-e2e.json` / `fullscreen-expanded.png` | 同一画布全屏的76件逐件验证、布局、按钮与 Esc 退出 |
| `nikon-f3hp-76-parts.html` | 真实发布页导出的独立离线 HTML |
| `persistence-audit.json` | 数据完整性、原发布/资产保留与费用零增长 |
| `before-update.sqlite3` / `draft-before.json` | 更新前在线数据库快照及草稿副本 |
| `source-ui-launcher-checks.json` | 启动脚本 21 项临时隔离检查；另外真实启动/重复启动/status 已通过 |

SQLite 快照、密文、测试资料和离线导出不提交到 Git。本报告与源码不含密钥。

## 最终 Linux 镜像

新镜像 `everything-manual:nikon76-20261004` 内嵌本轮全部分件、全屏及离线导出前端，运行时不依赖 Node。Linux AMD64 ABI 在 ARM Lima 虚拟机的 Rosetta 环境构建和运行；没有把该结果写作物理 AMD64 或 Linux ARM64 原生验收。

| 内容 | 值 |
| --- | --- |
| 镜像 payload manifest | `e5e60a0b6d68d8613972e87214e730c56e228aea1c6428866ffc7895e887de61` |
| 镜像 index | `09ac884e1096fc96887de4344e4a442b3c8f8553f52ce1bb87b83150feaa0135` |
| 镜像 config | `98b0c5e42e91d46b7d174e1d9de5bfe961a5b6542343137d754ba517ddab7dd8` |
| Linux 静态二进制 SHA-256 | `a4dcae88a76fca55f0395ad5da642a0478a89665321552e3c297d353d6ba6de9` |
| 镜像归档 | `delivery/everything-manual-nikon76-20261004.docker.tar.gz`，16,554,010 字节 |
| 归档 SHA-256 | `7f09e55dc6a50dcfb0e78371aae0d80307305268b9a7d478b0548338ae9dcb3b` |

独立 Compose 使用统一资料库的在线副本，仅为验收更换其独立登录凭据，不复制 API 密文、主密钥或浏览器会话。363 个 blob 的 SHA 校验通过。非 root UID 501、只读根文件系统、无 Node/npm/Rust/Python、Linux `--help`、新目录 `init/check` 和运行容器 `check` 均通过。供应商未配置，内部网络不提供外网路由；浏览器仅通过临时 localhost 转发读取此副本，不操作活动 8080/5173。

真实 Google Chrome 在该生产内嵌页面逐件验证 76 个编号的选择、展开、复原、全部展开/复原、搜索和离线导出，验证单个 Canvas 的全屏、按钮与 Esc 退出以及实际画面变化。生产 SPA 不暴露开发版新增的节点矩阵读取桥。没有远程请求或浏览器错误。生产 UI 证据为 `delivery/chrome-production.json`，完整摘要为 `delivery/summary.json`；构建日志、镜像元数据、运行检查、源码指纹和归档校验均保存在同一 `delivery/` 目录。临时 Compose、私有 tmpfs、QA 网络与 SSH 转发均已关闭；新镜像、归档和活动本机预览保留。

可在自己的 Docker Engine 导入上述归档：

```sh
docker load -i var/preview/validation/nikon-parts-20261004/delivery/everything-manual-nikon76-20261004.docker.tar.gz
```

正式 Compose 的默认镜像名仍为 `everything-manual:local`，从当前源码启动会重新构建包含本次改动的镜像；此具名镜像保留此次验证的构建快照。
