"""自由配置点の Sibson 自然近傍補間。格子・固定近傍数を使わない。"""
import ctypes
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay


def circumcircles(xy,tri,block=32768):
    result=np.empty((len(tri),3),dtype=np.float64)
    for first in range(0,len(tri),block):
        p=xy[tri[first:first+block]];a=p[:,1]-p[:,0];b=p[:,2]-p[:,0]
        rhs=np.column_stack((np.sum(a*a,axis=1),np.sum(b*b,axis=1)))*.5
        relative=np.linalg.solve(np.stack((a,b),axis=1),rhs[...,None])[...,0]
        result[first:first+len(p),:2]=p[:,0]+relative
        result[first:first+len(p),2]=np.sum(relative*relative,axis=1)
    return result


class SibsonSurface:
    def __init__(self,points,library="build/sibson_eval.dll"):
        self.xy=np.ascontiguousarray(points[:,:2],dtype=np.float64)
        self.z=np.ascontiguousarray(points[:,2],dtype=np.float64)
        self.lower=self.xy.min(axis=0)
        self.upper=self.xy.max(axis=0)
        self.triangulation=Delaunay(self.xy)
        self.tri=np.ascontiguousarray(self.triangulation.simplices,dtype=np.int32)
        self.adj=np.ascontiguousarray(self.triangulation.neighbors,dtype=np.int32)
        # 外接円は三角形ごとに独立。全数分の作業配列を同時に確保しない。
        self.circles=circumcircles(self.xy,self.tri)
        off,links=self.triangulation.vertex_neighbor_vertices
        self.off=np.ascontiguousarray(off,dtype=np.int32);self.links=np.ascontiguousarray(links,dtype=np.int32)
        self.lib=ctypes.CDLL(str(Path(library).resolve()))
        doubles=np.ctypeslib.ndpointer(dtype=np.float64,flags='C_CONTIGUOUS')
        ints=np.ctypeslib.ndpointer(dtype=np.int32,flags='C_CONTIGUOUS')
        self.lib.sibson.argtypes=[doubles,doubles,ints,ints,doubles,ints,ints,doubles,ints,ctypes.c_int,doubles]
        self.lib.sibson.restype=ctypes.c_int

    def __call__(self,query):
        q=np.ascontiguousarray(query,dtype=np.float64)
        starts=np.ascontiguousarray(self.triangulation.find_simplex(q),dtype=np.int32)
        if np.any(starts<0):raise ValueError('制御点の凸包外です')
        out=np.empty(len(q),dtype=np.float64)
        # 凸包上の自然近傍座標は辺の線形座標に一致する。
        # 地域全体だけでなく、独立タイルの矩形の外周も扱う。
        boundary=np.any((q<=self.lower+1e-8)|(q>=self.upper-1e-8),axis=1)
        if boundary.any():
            ids=starts[boundary];t=self.triangulation.transform[ids]
            bary=np.einsum('nij,nj->ni',t[:,:2,:],q[boundary]-t[:,2,:])
            w=np.column_stack((bary,1-bary.sum(axis=1)))
            out[boundary]=np.sum(w*self.z[self.tri[ids]],axis=1)
        ids=np.flatnonzero(~boundary)
        for first in range(0,len(ids),32768):
            selected=ids[first:first+32768];values=np.empty(len(selected))
            code=self._evaluate_native(np.ascontiguousarray(q[selected]),np.ascontiguousarray(starts[selected]),values)
            if code:raise ValueError(f'自然近傍の多角形計算に失敗: {selected[code-1]}')
            out[selected]=values
        return out

    def _evaluate_native(self,q,starts,values):
        return self.lib.sibson(self.xy,self.z,self.tri,self.adj,self.circles,self.off,self.links,q,starts,len(q),values)
