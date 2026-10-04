"""由自动绑定结果生成交互（动作 / 姿势）——不手工指定节点。

规则：
- 节点来自自动绑定（图例名称 → 部件 → 命中的分件节点）；枢轴、方向由该节点包围盒推导；
- 相机类：按图例名称里的关键词匹配典型操作（盖子/电池盖 → 外翻取下，按钮 → 按下，转盘 → 转动，拨杆 → 扳动）；
- 四足机器人：按包围盒找出四条腿（y 跨度大、靠四角），区分大腿/小腿，生成站立/趴下/坐下/握手/作揖；
- 所有动作只是外观示意，界面如实说明；结果以 candidate/auto 状态写入，需人工复核。
"""
import uuid
import numpy as np

Z = [0, 0, 1]


def rot(nodes, pivot, axis, angle):
    return {"nodes": nodes, "kind": "rotate", "pivot": [round(v, 4) for v in pivot], "axis": axis, "angleDeg": angle}


def tr(nodes, v):
    return {"nodes": nodes, "kind": "translate", "vector": [round(x, 4) for x in v]}


def _center(b):
    return [(b[0][k] + b[1][k]) / 2 for k in range(3)]


def _steps_for(K, keywords, limit=6):
    """K 可以是合并知识（含 steps）或草稿外壳（knowledge.steps）。"""
    steps = K.get("steps") if "steps" in K else K.get("knowledge", {}).get("steps", [])
    return [s["id"] for s in steps if any(k in s["title"] for k in keywords)][:limit]


def camera_actions(label, nodes, bounds, K):
    """label: partId → 图例名称；nodes: partId → [节点]（自动绑定）。"""
    allb = np.array([b for pair in bounds.values() for b in pair])
    lo, hi = allb.min(0), allb.max(0)
    actions = []
    used = set()
    for pid, name in label.items():
        ns = [n for n in nodes.get(pid, []) if n in bounds]
        if not ns:
            continue
        # 只移动"小件"：占整机包围盒体积过大的节点（机身）不参与动作
        ns = [n for n in ns if np.prod(np.array(bounds[n][1]) - np.array(bounds[n][0])) < 0.25 * np.prod(hi - lo)]
        ns = [n for n in ns if n not in used]
        if not ns:
            continue
        b = (np.min([bounds[n][0] for n in ns], 0), np.max([bounds[n][1] for n in ns], 0))
        c = _center(b)
        if any(k in name for k in ("电池盖", "手柄")):
            side = -1 if c[0] < 0 else 1
            hinge = [b[1][0] if side < 0 else b[0][0], b[0][1], b[1][2]]
            actions.append({"id": "open-battery", "label": f"取下{name}", "description": "露出电池仓（外观示意）。", "triggerPartIds": [pid], "mode": "toggle",
                            "durationMs": 900, "steps": [tr(ns, [side * 0.18, -0.05, 0.08]), rot(ns, hinge, [0, 1, 0], side * 25)], "stepIds": _steps_for(K, ("电池",))})
        elif "后盖" in name:
            hinge = [b[1][0], c[1], b[0][2]]
            actions.append({"id": "open-back", "label": f"打开{name}", "description": "装入胶片时打开（外观示意）。", "triggerPartIds": [pid], "mode": "toggle",
                            "durationMs": 1000, "steps": [rot(ns, hinge, [0, 1, 0], -70)], "stepIds": _steps_for(K, ("胶片",))})
        elif "按钮" in name or "快门" in name:
            actions.append({"id": f"press-{len(actions)}", "label": f"按下{name}", "description": None, "triggerPartIds": [pid], "mode": "pulse",
                            "durationMs": 300, "steps": [tr(ns, [0, -0.012, 0])], "stepIds": _steps_for(K, (name[:2],))})
        elif "转盘" in name or "拨盘" in name:
            actions.append({"id": f"turn-{len(actions)}", "label": f"转动{name}", "description": None, "triggerPartIds": [pid], "mode": "pulse",
                            "durationMs": 900, "steps": [rot(ns, c, [0, 1, 0], 60)], "stepIds": _steps_for(K, (name[:2],))})
        elif "杆" in name:
            pivot = [b[0][0] + 0.03, c[1], c[2]]
            actions.append({"id": f"lever-{len(actions)}", "label": f"扳动{name}", "description": None, "triggerPartIds": [pid], "mode": "pulse",
                            "durationMs": 700, "steps": [rot(ns, pivot, [0, 1, 0], -35)], "stepIds": _steps_for(K, (name[:2],))})
        else:
            continue
        used.update(ns)
    return actions[:12], []


def body_frame(bounds):
    """身体朝向：水平方向里跨度更大的轴是身体轴；最高的大件所在一端是头。
    返回 (body_axis_index, head_sign, swing_axis)。swing_axis = head × up，使正角度把下垂的小腿摆向头部。"""
    allb = np.array([b for pair in bounds.values() for b in pair]); lo, hi = allb.min(0), allb.max(0); span = hi - lo
    body = 0 if span[0] >= span[2] else 2
    tall = sorted(bounds.items(), key=lambda kv: -kv[1][1][1])[:3]
    head_c = np.mean([_center(b)[body] for _, b in tall])
    sign = 1.0 if head_c >= (lo[body] + hi[body]) / 2 else -1.0
    head = np.zeros(3); head[body] = sign
    swing = np.cross(head, [0.0, 1.0, 0.0])
    return body, sign, [float(round(v, 6)) for v in swing], lo, hi


def robot_legs(bounds):
    """四条腿：y 跨度大（上端到地面）且靠四角的节点为小腿；同一角落上方的节点为大腿；落地小件（足垫）跟随最近的小腿。"""
    body, sign, swing, lo, hi = body_frame(bounds); span = hi - lo; side = 2 if body == 0 else 0
    shins = [n for n, b in bounds.items() if (b[1][1] - b[0][1]) > 0.3 * span[1] and b[0][1] < lo[1] + 0.1 * span[1]]
    legs = {}
    for sh in shins:
        c = _center(bounds[sh])
        front = (c[body] - (lo[body] + hi[body]) / 2) * sign > 0
        corner = ("F" if front else "R") + ("L" if c[side] < (lo[side] + hi[side]) / 2 else "R")
        thighs = [n for n, b in bounds.items() if n != sh and abs(_center(b)[0] - c[0]) < 0.1 and abs(_center(b)[2] - c[2]) < 0.1
                  and _center(b)[1] > c[1] and (b[1][1] - b[0][1]) < 0.35 * span[1]]
        thigh = max(thighs, key=lambda n: np.prod(np.array(bounds[n][1]) - np.array(bounds[n][0]))) if thighs else None
        legs[corner] = (thigh, sh)
    feet = {}
    for n, b in bounds.items():
        if n in shins or (b[1][1] - b[0][1]) > 0.15 * span[1] or b[0][1] > lo[1] + 0.06 * span[1] or not legs:
            continue
        c = _center(b)
        near = min(legs.items(), key=lambda kv: np.hypot(_center(bounds[kv[1][1]])[0] - c[0], _center(bounds[kv[1][1]])[2] - c[2]))
        feet.setdefault(near[0], []).append(n)
    legs = {k: (th, sh, feet.get(k, [])) for k, (th, sh) in legs.items()}
    return legs, lo


def robot_poses(bounds, K, head_nodes):
    legs, _ = robot_legs(bounds)
    body, sign, AX, lo, hi = body_frame(bounds)
    ALL = list(bounds)

    def hip(n):
        b = bounds[n]; return [_center(b)[0], b[1][1] - 0.04, _center(b)[2]]

    def knee(n):
        b = bounds[n]; return [_center(b)[0], b[1][1] - 0.015, _center(b)[2]]

    def end_point(front, height):
        p = [0.0, lo[1] + height, 0.0]; p[body] = (hi[body] if (front and sign > 0) or (not front and sign < 0) else lo[body]) - sign * (0.2 if front else -0.15)
        return p

    def leg(corner, h, k):
        thigh, shin, feet = legs.get(corner, (None, None, []))
        if shin is None:
            return []
        lower = [shin] + feet
        if thigh is None:  # 分件把整条腿分成一块：只能整腿绕髋
            return [rot(lower, hip(shin), AX, h)]
        return [rot(lower, knee(shin), AX, k), rot([thigh] + lower, hip(thigh), AX, h)]

    def pose(pid, label, desc, ms, per, pitch=0.0, pivot=None):
        steps = []
        for corner, (h, k) in per.items():
            steps += leg(corner, h, k)
        if pitch:
            steps.append(rot(ALL, pivot, AX, pitch))
        return {"id": pid, "label": label, "description": desc, "durationMs": ms, "steps": steps or [rot(ALL[:1], [0, 0, 0], AX, 0)]}

    fold = {c: (-50, 115) for c in ("FL", "FR", "RL", "RR")}
    poses = [
        {"id": "stand", "label": "站立", "description": "默认站立姿态（模型原始姿态）。", "durationMs": 500, "steps": [rot(ALL[:1], [0, 0, 0], AX, 0)]},
        pose("lie-down", "趴下", "腿部收起、腹部贴地（开机前摆放姿态）。", 1200, fold),
        pose("sit", "坐下", "后腿收起、前腿支撑（语音指令“坐下”）。", 1100, {"RL": (-55, 120), "RR": (-55, 120), "FL": (10, 0), "FR": (10, 0)}, pitch=24, pivot=end_point(False, 0.3)),
        pose("shake-hand", "握手", "抬起右前腿（语音指令“握手”）。", 900, {"FR": (65, -20)}),
        pose("bow", "作揖", "前腿弯曲下压、臀部抬高（APP 动作“作揖/拜年”）。", 1100, {"FL": (-45, 110), "FR": (-45, 110)}, pitch=-16, pivot=end_point(True, 0.3)),
    ]
    actions = []
    if head_nodes:
        hb = (np.min([bounds[n][0] for n in head_nodes], 0), np.max([bounds[n][1] for n in head_nodes], 0))
        neck = list(_center(hb)); neck[1] = float(hb[0][1]); neck[body] -= sign * 0.06
        actions = [{"id": "nod", "label": "点头", "description": "头部上下点动（语音指令“点头”）。", "triggerPartIds": [], "mode": "pulse", "durationMs": 800, "steps": [rot(head_nodes, neck, AX, -18)], "stepIds": []},
                   {"id": "shake-head", "label": "摇头", "description": "头部左右摆动（语音指令“摇头”）。", "triggerPartIds": [], "mode": "pulse", "durationMs": 900, "steps": [rot(head_nodes, neck, [0, 1, 0], 25)], "stepIds": []}]
    return actions, poses


def ground(poses, seg_path, floor):
    from posepreview import apply
    from shade import load_parts
    parts = load_parts(seg_path)
    for p in poses:
        moved = apply(parts, p["steps"]); low = min(m.vertices[:, 1].min() for _, m in moved)
        if abs(floor - low) > 1e-3:
            p["steps"].append(tr([n for n, _ in parts], [0, floor - low, 0]))
    return poses


def build(key, label, nodes, bounds, K, seg_path=None):
    if key == "pentax":
        return camera_actions(label, nodes, bounds, K)
    body, sign, _, lo, hi = body_frame(bounds); span = hi - lo
    # 头部：最高处、位于头端的节点（不含腿）
    head = [n for n, b in bounds.items() if _center(b)[1] > lo[1] + 0.7 * span[1] and (_center(b)[body] - (lo[body] + hi[body]) / 2) * sign > 0.25 * span[body]]
    actions, poses = robot_poses(bounds, K, head)
    if seg_path:
        poses = ground(poses, seg_path, float(lo[1]))
    return actions, poses


def confirm_and_publish(api, item, draft):
    """演示用：把候选热点/自动绑定确认，并按发布不变量补齐复核声明后发布。真实使用应在复核页逐个核对。"""
    _, j, et = api.call("GET", f"/items/{item}/drafts/{draft}"); K = j["data"]["knowledge"]
    ups = [{"id": h["id"], "partId": h["partId"], "status": "confirmed", "anchor": h["anchor"]} for h in K["hotspots"] if h["status"] == "candidate"]
    _, j, et = api.call("PATCH", f"/items/{item}/drafts/{draft}", {"hotspots": {"upsert": ups}}, headers={"If-Match": et})
    binds = [{**b, "status": "confirmed"} for b in j["data"]["knowledge"]["interactive"]["bindings"]]
    _, j, et = api.call("PATCH", f"/items/{item}/drafts/{draft}", {"interactive": {"bindings": binds}}, headers={"If-Match": et})
    K = j["data"]["knowledge"]; hot = {h["partId"] for h in K["hotspots"] if h["status"] == "confirmed"}
    ents = {p["id"]: ({"reviewStatus": "confirmed"} if p["id"] in hot else {"reviewStatus": "confirmed", "textOnly": True}) for p in K["knowledge"]["parts"]}
    for sec in ("steps", "specs"):
        for e in K["knowledge"][sec]:
            ents[e["id"]] = {"reviewStatus": "confirmed"}
    _, j, et = api.call("PATCH", f"/items/{item}/drafts/{draft}", {"entities": ents}, headers={"If-Match": et})
    _, j, et = api.call("PATCH", f"/items/{item}/drafts/{draft}", {"modelReview": {"loaded": True, "userConfirmed": True}}, headers={"If-Match": et})
    _, r, _ = api.call("POST", f"/items/{item}/drafts/{draft}/publish", headers={"If-Match": et, "Idempotency-Key": str(uuid.uuid4())})
    return r["data"]["id"]
