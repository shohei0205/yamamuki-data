"""Bスプライン係数の削除順・滑らかさ・保存データの復元を検証する。"""
import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
from scipy.sparse import csr_matrix
import prune_ridge_spline as p
from ridge_spline import design,SPAN

class PruneTests(unittest.TestCase):
    def test_knot_refinement_preserves_surface_and_lpf_preserves_plane(self):
        rng=np.random.default_rng(364);c=rng.normal(size=(13,13));r=p.refine_matrix(8,32);xy=rng.uniform(-16000,16000,(1000,2))
        np.testing.assert_allclose(design(xy,8)@c.ravel(),design(xy,32)@(r@c@r.T).ravel(),atol=1e-12)
        y,x=np.indices((37,37));plane=500+.3*x-.7*y
        for sigma in [.5,1.,2.]:np.testing.assert_allclose(p.blur_coefficients(plane,sigma),plane,atol=1e-10)

    def test_greedy_matches_global_exhaustive_priority(self):
        n=8;m,k=p.gram_1d(n);length=SPAN/n*2;neighbors,gram=p.gram_neighbors(m,k,length)
        matrix=np.kron(m,m)+length**2*(np.kron(k,m)+np.kron(m,k))
        from_neighbors=csr_matrix((gram.ravel(),(np.repeat(np.arange((n+5)**2),49),neighbors.ravel())),shape=matrix.shape).toarray()
        np.testing.assert_allclose(matrix,from_neighbors,rtol=1e-14,atol=1e-8)
        rng=np.random.default_rng(135);detail=rng.normal(size=(n+5,n+5));trace=[];ix,info=p.prune(detail,.5,neighbors,gram,trace)
        c=detail.ravel();active=(abs(c)>1e-7)&(np.diag(matrix)>1e-20);removed=np.zeros_like(c)
        for index in trace:
            cost=c*c*np.diag(matrix)+2*c*(matrix@removed);cost[~active]=np.inf
            self.assertAlmostEqual(cost[index],np.min(cost),delta=max(1e-6,abs(cost[index])*1e-10))
            active[index]=False;removed[index]=c[index]
        np.testing.assert_array_equal(ix,np.flatnonzero(active))
        values,iterations=p.refit_details(detail,ix,csr_matrix(matrix));before=detail.ravel().copy();before[ix]=0;after=detail.ravel().copy();after[ix]-=values
        self.assertLessEqual(after@matrix@after,before@matrix@before+1e-5)

    def test_continuous_basis_has_no_isolated_fine_details(self):
        with sqlite3.connect('build/spline-pruning-continuous-v1/pruned.sqlite3') as db:rows=db.execute('SELECT * FROM surfaces WHERE keep<1').fetchall()
        self.assertEqual(len(rows),12)
        for region,keep,raw,bb,ib,cb in rows:
            info=json.loads(raw);base=np.frombuffer(zlib.decompress(bb),'<f4').reshape(info['baseN']+5,-1)
            ix=p.ungaps(zlib.decompress(ib));values=np.frombuffer(zlib.decompress(cb),'<f4')
            self.assertEqual(len(ix),0);self.assertEqual(len(values),0)
            self.assertLessEqual(base.size,1369+int(info['initialDetails']*keep))
            dense=p.reconstruct(base,ix,values,256);r=p.refine_matrix(info['baseN'],256)
            np.testing.assert_allclose(dense,r@base.astype(float)@r.T,atol=.001)
        # 細格子へ展開した粗い曲面は、粗い基底への再近似でも変わらない。
        rng=np.random.default_rng(761);base=rng.normal(size=(13,13));r=p.refine_matrix(8,32);inverse=np.linalg.pinv(r)
        np.testing.assert_allclose(inverse@(r@base@r.T)@inverse.T,base,atol=1e-12)

    def test_saved_models_restore_from_blobs(self):
        rng=np.random.default_rng(635);xy=np.vstack((rng.uniform(-16000,16000,(2048,2)),[[-16000,-16000],[16000,16000]]));a=design(xy,256)
        with sqlite3.connect('build/spline-pruning-v2/pruned.sqlite3') as db:rows=db.execute('SELECT * FROM surfaces').fetchall()
        self.assertEqual(len(rows),15);report=[]
        for region,keep,raw,bb,ib,cb in rows:
            info=json.loads(raw);base=np.frombuffer(zlib.decompress(bb),'<f4').reshape(37,37);ix=p.ungaps(zlib.decompress(ib));values=np.frombuffer(zlib.decompress(cb),'<f4')
            self.assertEqual(len(ix),info['retainedDetails']);self.assertTrue(np.all(np.diff(ix)>0));self.assertEqual(len(bb)+len(ib)+len(cb),info['cumulativeBytes'])
            dense=p.reconstruct(base,ix,values,256);h=a@dense.ravel();self.assertTrue(np.all(np.isfinite(h)))
            with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as source:b=source.execute('SELECT coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()[0]
            reference=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261)
            if keep==1:np.testing.assert_allclose(h,a@reference.ravel(),atol=.001)
            else:
                self.assertLess(info['cumulativeBytes'],info['originalBytes']);self.assertTrue(info['refit']);self.assertGreater(info['refitIterations'],0)
                # 高さだけでなく、実際の二乗高さ＋傾きの積分目的が再調整で下がること。
                target=p.blur_coefficients(reference.astype(float),info['sigmaMeters']/125);r=p.refine_matrix(32,256);detail=target-r@base.astype(float)@r.T
                before=detail.copy();before.ravel()[ix]-=detail.ravel()[ix];after=detail.copy();after.ravel()[ix]-=values
                m,k=p.gram_1d(256)
                def energy(e):return float(np.sum(e*(m@e@m.T+250**2*(k@e@m.T+m@e@k.T))))
                self.assertLessEqual(energy(after),energy(before)*(1+1e-6)+.01)
            report.append(dict(region=region,keep=keep,bytes=info['cumulativeBytes'],heightRmse=info['heightRmse']))
        Path('build/spline-pruning-v2/numeric-verification.json').write_text(json.dumps(dict(passed=True,models=report),indent=2),encoding='utf-8')

if __name__=='__main__':unittest.main()
