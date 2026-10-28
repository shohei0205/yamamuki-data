import base64
import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
from scipy.sparse import eye
from scipy.spatial import cKDTree
from ridge_surface import subdivide,preview,flow_lines,reference_mesh_sizes
from shapely.geometry import LineString,Point
from shapely.strtree import STRtree


class RidgeTests(unittest.TestCase):
    def test_local_segment_candidates_match_full_scan(self):
        rng=np.random.default_rng(5);xy=rng.uniform(-16000,16000,(1000,2))
        xy=np.vstack([xy,[[0,0],[1000,0],[250,0],[500,0],[750,0],[500,.00001]]])
        tree=cKDTree(xy)
        for a,b in [(1000,1001),(3,73),(1002,1004)]:
            d=xy[b]-xy[a];den=d@d
            def valid(ids):
                delta=xy[ids]-xy[a];t=(delta[:,0]*d[0]+delta[:,1]*d[1])/den
                distance=np.linalg.norm(xy[ids]-(xy[a]+np.clip(t,0,1)[:,None]*d),axis=1)
                return ids[(distance<1e-4)&(t>1e-7)&(t<1-1e-7)]
            local=np.array(sorted(tree.query_ball_point((xy[a]+xy[b])*.5,np.sqrt(den)*.5+1e-4)),int)
            np.testing.assert_array_equal(valid(local),valid(np.arange(len(xy))))

    def test_indexed_skeleton_classification(self):
        lines=[LineString([[0,0],[2,2]]),LineString([[0,2],[2,0]]),LineString([[0,0],[2,0]])]
        tree=STRtree(lines)
        for xy in [[0,0],[1,1],[1,.5],[2,2],[.2,1.9]]:
            point=Point(xy)
            expected=min(range(len(lines)),key=lambda i:lines[i].distance(point))
            actual=int(np.min(tree.query_nearest(point,all_matches=True)))
            self.assertEqual(expected,actual)

    def test_soft_crease_preserves_peaks_and_plane(self):
        xy=np.array([[0,0],[1,0],[2,0],[0,1],[1,1],[2,1]],float)
        faces=[[0,1,4,3],[1,2,5,4]]
        creases=[[0,1,0],[1,2,0],[2,5,0],[5,4,0],[4,3,0],[3,0,0],[1,4,1]]
        corners=[0,1,2,3,4,5];h=np.array([0,1,0,0,1,0.])
        result=[]
        for strength in [0.,.25,1.]:
            xx,ff,cc,kk,m=subdivide(xy,faces,creases,corners,eye(6,format='csr'),strength)
            plane=100+xy[:,0]*.3-xy[:,1]*.2
            np.testing.assert_allclose(m@plane,100+xx[:,0]*.3-xx[:,1]*.2,atol=1e-8)
            np.testing.assert_allclose((m@h)[corners],h[corners])
            v=np.flatnonzero(np.all(np.isclose(xx,[1,.5]),axis=1))[0]
            result.append((m@h)[v])
        self.assertLess(result[0],result[1]);self.assertLess(result[1],result[2])

    def test_reference_mesh_sizes_follow_level(self):
        value=reference_mesh_sizes('build/lpf-curve-smooth-prototype/comparison.sqlite3','fuji')
        self.assertEqual(set(value['bytesByZ']),set(range(6,15)))
        self.assertEqual(value['bytesByZ'][14],557701)
        self.assertLess(value['bytesByZ'][6],value['bytesByZ'][10])
        self.assertLess(value['bytesByZ'][10],value['bytesByZ'][14])

    def test_plane_and_protected_points(self):
        xy=np.array([[0,0],[2,0],[4,0],[0,2],[2.3,2.2],[4,2],[0,4],[2,4],[4,4]],float)
        faces=[[0,1,4,3],[1,2,5,4],[3,4,7,6],[4,5,8,7]]
        creases=[[0,1,0],[1,2,0],[2,5,0],[5,8,0],[8,7,0],[7,6,0],[6,3,0],[3,0,0],[1,4,1],[4,7,1]];corners=[0,2,6,8,4]
        h=100+xy[:,0]*.5-xy[:,1]*.7;matrix=eye(len(xy),format='csr')
        for _ in range(3):
            xy,faces,creases,corners,matrix=subdivide(xy,faces,creases,corners,matrix)
            np.testing.assert_allclose(matrix@h,100+xy[:,0]*.5-xy[:,1]*.7,atol=1e-8)
            np.testing.assert_allclose(np.asarray(matrix.sum(axis=1)).ravel(),1,atol=1e-8)
            for v in corners:self.assertAlmostEqual(float((matrix@h)[v]),h[v])

    def test_flow_candidates_follow_height(self):
        axis=np.linspace(-1,1,33);x,y=np.meshgrid(axis,axis);h=1000-500*x*x-300*y*y
        ridge=flow_lines(h,8,1);valley=flow_lines(h,8,2)
        self.assertTrue(ridge);self.assertTrue(valley)
        for line in ridge:self.assertTrue((np.diff(line[:,2])>=-1e-8).all())
        for line in valley:self.assertTrue((np.diff(line[:,2])<=1e-8).all())

    def test_preview_stencils_and_crease_normals(self):
        model=dict(vertices=[[0,0,0],[1,0,1],[2,0,0],[0,1,0],[1,1,1],[2,1,0]],faces=[[0,1,4,3],[1,2,5,4]],creases=[[0,1,0],[1,2,0],[2,5,0],[5,4,0],[4,3,0],[3,0,0],[1,4,1]],corners=[0,2,3,5],span=2)
        m=preview(model,2);weights=np.frombuffer(base64.b64decode(m['weights']),'<f4').reshape(-1,4);lookup=np.frombuffer(base64.b64decode(m['lookup']),'<u4').reshape(-1,2);xy=np.frombuffer(base64.b64decode(m['xy']),'<f4').reshape(-1,2)
        control=np.array(model['vertices'])[:,2];values=[];gradients=[]
        for start,count in lookup:
            w=weights[start:start+count];values.append(w[:,1]@control[w[:,0].astype(int)]);gradients.append(w[:,2]@control[w[:,0].astype(int)])
            self.assertAlmostEqual(float(w[:,1].sum()),1,places=5);self.assertLess(abs(float(w[:,2].sum())),1e-5);self.assertLess(abs(float(w[:,3].sum())),1e-5)
        v=np.flatnonzero(np.all(np.isclose(xy,[1,.5]),axis=1));self.assertEqual(len(v),2);self.assertAlmostEqual(values[v[0]],values[v[1]],places=6);self.assertGreater(abs(gradients[v[0]]-gradients[v[1]]),1.)

    def test_generated_control_topology(self):
        from lpf_curve_compare import Source
        path=Path('build/ridge-surface-prototype-v4/ridge.sqlite3');source=Source('build/terrain-lod.sqlite3')
        with sqlite3.connect('build/lpf-curve-smooth-prototype/comparison.sqlite3') as metadata:
            centers={name:np.array(json.loads(raw)['center']) for name,raw in metadata.execute('SELECT name,info FROM regions')}
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            rows=db.execute('SELECT region,quality,model FROM surfaces').fetchall();self.assertEqual(len(rows),9)
            for region,quality,raw in rows:
                model=json.loads(zlib.decompress(raw));vertices=np.array(model['vertices']);edges={}
                for face in model['faces']:
                    self.assertIn(len(face),[3,4])
                    for a,b in zip(face,face[1:]+face[:1]):edges[tuple(sorted((a,b)))]=edges.get(tuple(sorted((a,b))),0)+1
                self.assertTrue(all(v<=2 for v in edges.values()))
                self.assertTrue(all(tuple(sorted((a,b))) in edges for a,b,_ in model['creases']))
                protected=np.unique(np.r_[np.array(model['creases'])[:,:2].ravel(),model['corners']]).astype(int)
                expected=source.sample(vertices[protected,0]+centers[region][0],vertices[protected,1]+centers[region][1])
                np.testing.assert_allclose(vertices[protected,2],expected,atol=.005)
                if quality=='medium':
                    mesh=preview(model,4);xy=np.frombuffer(base64.b64decode(mesh['xy']),'<f4').reshape(-1,2);ix=np.frombuffer(base64.b64decode(mesh['indices']),'<u4').reshape(-1,3);p=xy[ix]
                    area=(p[:,1,0]-p[:,0,0])*(p[:,2,1]-p[:,0,1])-(p[:,1,1]-p[:,0,1])*(p[:,2,0]-p[:,0,0])
                    self.assertTrue((area>0).all())


if __name__=='__main__':unittest.main()
