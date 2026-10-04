import json, numpy as np, trimesh
from PIL import Image, ImageDraw
from align import drawing_mask, find_view, rot, pick_points
def load_seg(path):
    sc=trimesh.load(path); meshes=[]; nof=[]
    for n in sc.graph.nodes_geometry:
        T,g=sc.graph[n]; m=sc.geometry[g].copy(); m.apply_transform(T); meshes.append(m); nof+=[n]*len(m.faces)
    return trimesh.util.concatenate(meshes), nof
def clean_for_mask(img, points, r=0):
    return img
def bind_view(seg_path, drawing_img, points, grid=None, label="view"):
    """drawing_img: PIL image of the (line-art) view; points: [(x,y)] in drawing_img pixels."""
    mesh,nof=load_seg(seg_path)
    dm,dbox=drawing_mask(drawing_img)
    best=find_view(mesh,dm,grid)
    hits=pick_points(mesh,rot(*best[1]),dbox,points,nof)
    return {"view":best[1],"iou":float(best[0]),"hits":hits}
