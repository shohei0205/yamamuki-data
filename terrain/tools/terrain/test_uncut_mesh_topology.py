import unittest, struct, zlib
import numpy as np
from scipy.spatial import Delaunay
from weighted_mesh_allocation import split_mesh
from uncut_mesh_topology import restore, tile_topology


class UncutTopologyTests(unittest.TestCase):
    def test_removes_cut_points_without_changing_surface(self):
        rng=np.random.default_rng(12)
        xy=np.vstack([[[0,0],[16000,0],[16000,16000],[0,16000]],rng.uniform(0,16000,(50,2))])
        p=np.column_stack([xy,100+400*np.sin(xy[:,0]/4000)*np.cos(xy[:,1]/5000)])
        delaunay=Delaunay(xy);f=delaunay.simplices.copy()
        q=p[f];negative=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])[:,2]<0;f[negative]=f[negative][:,[0,2,1]]
        parts=split_mesh(p,f,width=16000);points=[];faces=[];lookup={}
        for (x,y),part in parts.items():
            ids=[]
            for v in part['points']:
                world=v+[x*8000,y*8000,0];key=tuple(np.rint(world[:2]*256).astype(int))
                if key not in lookup:lookup[key]=len(points);points.append(world)
                ids.append(lookup[key])
            faces.extend(np.asarray(ids)[part['faces']])
        original={tuple(np.rint(v[:2]*256).astype(int)) for v in p}
        candidates=[v for k,v in lookup.items() if k not in original]
        points=np.asarray(points);faces=np.asarray(faces)
        recovered,removed=restore(points,faces,candidates)
        self.assertGreater(removed,len(candidates)*.8)
        q=points[recovered];self.assertAlmostEqual(np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])[:,2].sum()/2,16000**2,delta=.05)
        samples=q.mean(axis=1);tri=delaunay.find_simplex(samples[:,:2]);self.assertTrue((tri>=0).all())
        transform=delaunay.transform[tri];w=np.einsum('nij,nj->ni',transform[:,:2],samples[:,:2]-transform[:,2]);weights=np.column_stack([w,1-w.sum(axis=1)])
        expected=(p[delaunay.simplices[tri],2]*weights).sum(axis=1)
        self.assertLess(np.abs(samples[:,2]-expected).max(),.02)
        records=dict((k,body) for k,body,_ in tile_topology(points,recovered))
        groups={};shared=0
        for (x,y),body in records.items():
            raw=zlib.decompress(body);_,n,nv,ni,core=struct.unpack_from('<4sIIII',raw)
            p2=np.frombuffer(raw,'<f4',nv*3,20).reshape(-1,3).astype(float);p2[:,:2]+=[x*8000,y*8000]
            normals=np.frombuffer(raw,'<f4',nv*3,20+nv*12+ni*4).reshape(-1,3)
            for v,normal in zip(p2,normals):
                key=tuple(np.rint(v*np.array([256,256,1024])).astype(int))
                if key in groups:np.testing.assert_allclose(normal,groups[key],atol=1e-7);shared+=1
                groups[key]=normal
        self.assertGreater(shared,10)


if __name__=='__main__':unittest.main()
