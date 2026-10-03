import numpy as np, trimesh
from PIL import Image, ImageDraw
from align import rot
PAL=[(230,25,75),(60,180,75),(255,225,25),(0,130,200),(245,130,48),(145,30,180),(70,240,240),(240,50,230),(210,245,60),(250,190,212),(0,128,128),(220,190,255),(170,110,40),(255,250,200),(128,0,0),(170,255,195),(128,128,0),(255,215,180),(0,0,128),(128,128,128)]
def load_parts(path):
    sc=trimesh.load(path); out=[]
    for n in sc.graph.nodes_geometry:
        T,g=sc.graph[n]; m=sc.geometry[g].copy(); m.apply_transform(T); out.append((n,m))
    return out
def shade(parts,R,size=600,points=None,labels=True):
    allV=np.vstack([m.vertices for _,m in parts])@R.T; lo=allV[:,:2].min(0); hi=allV[:,:2].max(0); s=(size-20)/max(hi-lo); off=(size-(hi-lo)*s)/2
    proj=lambda P:np.stack([(P[:,0]-lo[0])*s+off[0], size-((P[:,1]-lo[1])*s+off[1])],1)
    img=Image.new("RGB",(size,size),(240,240,240)); d=ImageDraw.Draw(img); tris=[]
    L=np.array([0.3,0.6,0.75]); L/=np.linalg.norm(L)
    for k,(n,m) in enumerate(parts):
        V=m.vertices@R.T; N=m.face_normals@R.T; vis=N[:,2]>0; F=m.faces[vis]; Q=proj(V)
        for f,nm in zip(F,N[vis]): tris.append((V[f][:,2].mean(),k,Q[f],max(0.3,float(nm@L))))
    tris.sort(key=lambda t:t[0])
    for z,k,q,sh in tris: d.polygon([tuple(p) for p in q],fill=tuple(int(c*sh) for c in PAL[k%len(PAL)]))
    if points:
        for lab,P in points:
            q=proj((np.array([P])@R.T))[0]; d.ellipse([q[0]-7,q[1]-7,q[0]+7,q[1]+7],outline=(0,0,0),width=3); d.text((q[0]+9,q[1]-7),str(lab),fill=(0,0,0))
    return img
