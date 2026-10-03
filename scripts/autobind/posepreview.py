import json, numpy as np
from shade import load_parts, shade
from align import rot as vrot
from PIL import Image
def mat(step,t=1.0):
    M=np.eye(4)
    if step["kind"]=="translate": M[:3,3]=np.array(step["vector"])*t; return M
    ax=np.array(step["axis"],float); ax/=np.linalg.norm(ax); a=np.radians(step["angleDeg"])*t
    K=np.array([[0,-ax[2],ax[1]],[ax[2],0,-ax[0]],[-ax[1],ax[0],0]]); R=np.eye(3)+np.sin(a)*K+(1-np.cos(a))*K@K
    P=np.array(step["pivot"]); M[:3,:3]=R; M[:3,3]=P-R@P; return M
def compose(steps):
    out={}
    for st in steps:
        m=mat(st)
        for n in st["nodes"]: out[n]=m@out.get(n,np.eye(4))
    return out
def apply(parts,steps):
    mats=compose(steps); res=[]
    for n,m in parts:
        mm=m.copy()
        if n in mats: V=np.c_[mm.vertices,np.ones(len(mm.vertices))]@mats[n].T; mm.vertices=V[:,:3]
        res.append((n,mm))
    return res
