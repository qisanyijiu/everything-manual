"""把说明书线稿视角与 3D 模型对齐，并把线稿上的 2D 点反投影到模型表面。

做法：正交相机，搜索 yaw/pitch/roll（再细化），渲染模型轮廓掩膜，与线稿掩膜做 IoU；
最佳视角下对每个 2D 点做射线求交（trimesh），返回模型局部坐标与命中的分件节点。
"""
import numpy as np, trimesh
from PIL import Image, ImageDraw, ImageFilter, ImageOps

def drawing_mask(img, size=256):
    g=ImageOps.grayscale(img); a=np.array(g)<200
    ys,xs=np.nonzero(a); x0,x1,y0,y1=xs.min(),xs.max(),ys.min(),ys.max()
    # 线稿是线条：膨胀后填充轮廓内部
    m=Image.fromarray((a*255).astype(np.uint8)).filter(ImageFilter.MaxFilter(9))
    m=np.array(m)>0; filled=fill_holes(m)
    return filled,(x0,y0,x1,y1)

def fill_holes(m):
    h,w=m.shape; out=np.ones_like(m,bool); stack=[(0,0),(0,w-1),(h-1,0),(h-1,w-1)]; seen=np.zeros_like(m,bool)
    from collections import deque
    q=deque([p for p in stack if not m[p]])
    for p in q: seen[p]=True
    while q:
        y,x=q.popleft(); out[y,x]=False
        for dy,dx in ((1,0),(-1,0),(0,1),(0,-1)):
            ny,nx=y+dy,x+dx
            if 0<=ny<h and 0<=nx<w and not seen[ny,nx] and not m[ny,nx]: seen[ny,nx]=True; q.append((ny,nx))
    return out

def rot(yaw,pitch,roll):
    a,b,c=np.radians([yaw,pitch,roll])
    Ry=np.array([[np.cos(a),0,np.sin(a)],[0,1,0],[-np.sin(a),0,np.cos(a)]])
    Rx=np.array([[1,0,0],[0,np.cos(b),-np.sin(b)],[0,np.sin(b),np.cos(b)]])
    Rz=np.array([[np.cos(c),-np.sin(c),0],[np.sin(c),np.cos(c),0],[0,0,1]])
    return Rz@Rx@Ry

def model_mask(mesh,R,size=256):
    V=mesh.vertices@R.T; P=V[:,:2]; lo=P.min(0); hi=P.max(0); s=(size-8)/max(hi-lo); off=(size-(hi-lo)*s)/2
    img=Image.new("L",(size,size),0); d=ImageDraw.Draw(img)
    Q=(P-lo)*s+off; Q[:,1]=size-Q[:,1]
    for f in mesh.faces[::max(1,len(mesh.faces)//40000)]: d.polygon([tuple(Q[i]) for i in f],fill=255)
    return np.array(img)>0,(lo,s,off)

def fit_to_box(mask):
    ys,xs=np.nonzero(mask); return mask[ys.min():ys.max()+1,xs.min():xs.max()+1]

def iou_boxes(a,b,size=160):
    A=np.array(Image.fromarray(fit_to_box(a).astype(np.uint8)*255).resize((size,size)))>127
    B=np.array(Image.fromarray(fit_to_box(b).astype(np.uint8)*255).resize((size,size)))>127
    return (A&B).sum()/max(1,(A|B).sum())

def find_view(mesh,dmask,coarse=None):
    best=(-1,None)
    grid=coarse or [(y,p,0) for y in range(0,360,15) for p in range(-60,75,15)]
    for y,p,r in grid:
        m,_=model_mask(mesh,rot(y,p,r),160); s=iou_boxes(m,dmask)
        if s>best[0]: best=(s,(y,p,r))
    y0,p0,r0=best[1]
    for step in (6,3,1.5):
        improved=True
        while improved:
            improved=False
            for dy,dp,dr in ((step,0,0),(-step,0,0),(0,step,0),(0,-step,0),(0,0,step),(0,0,-step)):
                cand=(y0+dy,p0+dp,r0+dr); m,_=model_mask(mesh,rot(*cand),160); s=iou_boxes(m,dmask)
                if s>best[0]+1e-4: best=(s,cand); y0,p0,r0=cand; improved=True
    return best

def project_points(mesh,R,dbox,points,node_of_face=None):
    """points: drawing pixel coords. Map drawing bbox -> model projected bbox, cast rays along -view."""
    V=mesh.vertices@R.T; lo=V[:,:2].min(0); hi=V[:,:2].max(0)
    x0,y0,x1,y1=dbox; out=[]
    inter=trimesh.ray.ray_triangle.RayMeshIntersector(mesh)
    Rinv=R.T; zmax=V[:,2].max()+1
    for (px,py) in points:
        u=(px-x0)/max(1,x1-x0); v=(py-y0)/max(1,y1-y0)
        X=lo[0]+u*(hi[0]-lo[0]); Y=hi[1]-v*(hi[1]-lo[1])
        origin=Rinv@np.array([X,Y,zmax]); direction=Rinv@np.array([0,0,-1.0])
        locs,_,tri=inter.intersects_location([origin],[direction],multiple_hits=True)
        if len(locs)==0:
            # 落在轮廓外/缝隙：在屏幕平面上取最近的可见点
            out.append(None); continue
        k=np.argmax((locs@R.T)[:,2]); out.append({"point":locs[k].tolist(),"face":int(tri[k]),"node":None if node_of_face is None else node_of_face[int(tri[k])]})
    return out

def pick_points(mesh,R,dbox,points,node_of_face=None,size=1024):
    """Z-buffer 拾取：按拟合视角光栅化 face id，在 2D 点（及邻域）取最前面的面，再求面内的三维点。"""
    V=mesh.vertices@R.T; lo=V[:,:2].min(0); hi=V[:,:2].max(0)
    s=(size-8)/max(hi-lo); off=(size-(hi-lo)*s)/2
    Q=(V[:,:2]-lo)*s+off; Q[:,1]=size-Q[:,1]
    N=mesh.face_normals@R.T; order=np.argsort(V[mesh.faces][:,:,2].mean(1))
    img=Image.new("I",(size,size),-1); d=ImageDraw.Draw(img)
    for fi in order:
        if N[fi,2]<=0: continue
        d.polygon([tuple(Q[i]) for i in mesh.faces[fi]],fill=int(fi))
    ids=np.array(img)
    x0,y0,x1,y1=dbox; mlo=Q.min(0); mhi=Q.max(0); out=[]
    for (px,py) in points:
        u=(px-x0)/max(1,x1-x0); v=(py-y0)/max(1,y1-y0)
        X=mlo[0]+u*(mhi[0]-mlo[0]); Y=mlo[1]+v*(mhi[1]-mlo[1])
        fi=-1
        for r in range(0,14):
            ys,xs=np.mgrid[-r:r+1,-r:r+1]; cand=[(int(Y+dy),int(X+dx)) for dy,dx in zip(ys.ravel(),xs.ravel()) if 0<=int(Y+dy)<size and 0<=int(X+dx)<size]
            vals=[ids[c] for c in cand if ids[c]>=0]
            if vals: fi=max(set(vals),key=vals.count); break
        if fi<0: out.append(None); continue
        tri=V[mesh.faces[fi]]; q=Q[mesh.faces[fi]]
        # 面内重心坐标
        A=np.array([[q[0,0]-q[2,0],q[1,0]-q[2,0]],[q[0,1]-q[2,1],q[1,1]-q[2,1]]])
        try: l1,l2=np.linalg.solve(A,np.array([X-q[2,0],Y-q[2,1]]))
        except np.linalg.LinAlgError: l1=l2=1/3
        l1,l2=np.clip([l1,l2],0,1); l3=max(0,1-l1-l2); w=np.array([l1,l2,l3]); w/=w.sum()
        P=(mesh.vertices[mesh.faces[fi]]*w[:,None]).sum(0)
        out.append({"point":P.tolist(),"face":int(fi),"node":None if node_of_face is None else node_of_face[fi],"screen":[float(X),float(Y)]})
    return out

def snap_missing(mesh,R,dbox,points,hits,node_of_face,size=1024,max_px=80):
    """未命中的点（落在轮廓外，如标注圆圈画在腿旁）：取屏幕上最近的可见面。"""
    V=mesh.vertices@R.T; lo=V[:,:2].min(0); hi=V[:,:2].max(0); s=(size-8)/max(hi-lo); off=(size-(hi-lo)*s)/2
    Q=(V[:,:2]-lo)*s+off; Q[:,1]=size-Q[:,1]
    N=mesh.face_normals@R.T; vis=np.where(N[:,2]>0)[0]; C=Q[mesh.faces[vis]].mean(1); depth=V[mesh.faces[vis]][:,:,2].mean(1)
    x0,y0,x1,y1=dbox; mlo=Q.min(0); mhi=Q.max(0); out=[]
    for (px,py),h in zip(points,hits):
        if h is not None: out.append(h); continue
        X=mlo[0]+(px-x0)/max(1,x1-x0)*(mhi[0]-mlo[0]); Y=mlo[1]+(py-y0)/max(1,y1-y0)*(mhi[1]-mlo[1])
        d=np.hypot(C[:,0]-X,C[:,1]-Y); near=d<d.min()+6; cand=vis[near]; fi=cand[np.argmax(depth[near])]
        if d.min()>max_px: out.append(None); continue
        P=mesh.vertices[mesh.faces[fi]].mean(0)
        out.append({"point":P.tolist(),"face":int(fi),"node":node_of_face[fi],"screen":[float(X),float(Y)],"snapped":True})
    return out
