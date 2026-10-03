import json, numpy as np
B=json.load(open("cd_bounds.json"))
# 分件结果：左后腿 tripo_part_2 是整条腿（大腿+小腿一体），tripo_part_25 是髋部电机罩
legs={"FL":("tripo_part_4","tripo_part_13"),"FR":("tripo_part_8","tripo_part_23"),"RL":("tripo_part_25","tripo_part_2"),"RR":("tripo_part_6","tripo_part_3")}
RIGID={"RL"}  # 没有独立小腿：只能整腿绕髋转
FLOOR=min(b["min"][1] for b in B.values())
def mid(n,k): return round((B[n]["min"][k]+B[n]["max"][k])/2,4)
def hip(th): return [mid(th,0), round(B[th]["max"][1]-0.04,4), mid(th,2)]
def knee(sh): return [mid(sh,0), round(B[sh]["max"][1]-0.015,4), mid(sh,2)]
Z=[0,0,1]; ALL=list(B)
def rot(nodes,pivot,axis,angle): return {"nodes":nodes,"kind":"rotate","pivot":pivot,"axis":axis,"angleDeg":angle}
def tr(nodes,v): return {"nodes":nodes,"kind":"translate","vector":v}
def leg(name,hip_deg,knee_deg):
    th,sh=legs[name]
    if name in RIGID: return [rot([th,sh],hip(th),Z,hip_deg)]
    return [rot([sh],knee(sh),Z,knee_deg), rot([th,sh],hip(th),Z,hip_deg)]
def grounded(steps, parts):
    from posepreview import apply
    moved=apply(parts,steps); low=min(m.vertices[:,1].min() for _,m in moved)
    return steps+[tr(ALL,[0,round(FLOOR-low,4),0])] if abs(FLOOR-low)>1e-3 else steps
def pose(id,label,desc,ms,per_leg,parts,pitch=0.0,pitch_pivot=(0,0,0)):
    steps=[]
    for n,(h,k) in per_leg.items(): steps+=leg(n,h,k)
    if pitch: steps.append(rot(ALL,list(pitch_pivot),Z,pitch))
    return {"id":id,"label":label,"description":desc,"durationMs":ms,"steps":grounded(steps,parts)}
def poses(parts):
    # 角度约定：绕 +z；正角让向下的小腿朝 +x（机头方向）收起；负角让大腿下端向后摆。
    return [
     {"id":"stand","label":"站立","description":"默认站立姿态（模型原始姿态）。","durationMs":500,"steps":[rot([legs["FL"][0]],hip(legs["FL"][0]),Z,0)]},
     pose("lie-down","趴下","腿部收起、腹部贴地（开机前摆放姿态，见第9页）。",1200,{"FL":(-50,115),"FR":(-50,115),"RL":(-70,0),"RR":(-50,115)},parts),
     pose("sit","坐下","后腿收起、前腿伸直支撑（语音指令“坐下”）。",1100,{"RL":(-75,0),"RR":(-55,120),"FL":(10,0),"FR":(10,0)},parts,pitch=24,pitch_pivot=(-0.35,-0.1,0)),
     pose("shake-hand","握手","抬起右前腿（语音指令“握手”）。",900,{"FR":(65,-20)},parts),
     pose("bow","作揖","前腿弯曲下压、臀部抬高（APP 动作“作揖/拜年”）。",1100,{"FL":(-45,110),"FR":(-45,110)},parts,pitch=-16,pitch_pivot=(0.3,-0.2,0)),
    ]
