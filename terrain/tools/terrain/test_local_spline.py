"""局所的な階層細分化と保存曲面の復元を検証する。"""
import base64,json,sqlite3,unittest,zlib
import numpy as np
from scipy.sparse import eye,kron,csr_matrix
from local_spline import inside_basis,truncate,unpack,choose_domain
from nonuniform_spline import cardinal
from prune_ridge_spline import refine_matrix
class LocalTests(unittest.TestCase):
    def test_local_refinement_preserves_partition_of_unity(self):
        n=8;x=np.linspace(-16000,16000,101);e=kron(csr_matrix(cardinal(n,x)),csr_matrix(cardinal(n,x)),format='csr');used=np.asarray(e.sum(axis=0)).ravel()>0;t=eye(169,format='csr')[:,used]
        domain=np.zeros((16,16),bool);domain[4:12,4:12]=True;selected=inside_basis(domain)
        r=kron(csr_matrix(refine_matrix(8,16)),csr_matrix(refine_matrix(8,16)),format='csr');fine,_,_=truncate(t,r,selected)
        e=kron(csr_matrix(cardinal(16,x)),csr_matrix(cardinal(16,x)),format='csr')
        np.testing.assert_allclose((e@fine)@np.ones(fine.shape[1]),1,atol=1e-12)
        self.assertLess(selected.sum(),16**2)
        self.assertTrue(np.all(fine.data>=0))
    def test_diagonal_region_does_not_refine_whole_axes(self):
        x=np.linspace(-1,1,129);xx,yy=np.meshgrid(x,x);error=np.exp(-((xx-yy)/.08)**2)*np.exp(-(xx**2+yy**2)/.5)
        domain=choose_domain(error,64,np.ones((32,32),bool),1000)
        self.assertGreater(domain.sum(),0);self.assertLess(domain.sum(),4096)
        self.assertFalse(np.any(domain.all(axis=0)));self.assertFalse(np.any(domain.all(axis=1)))
    def test_graded_regions_merge_small_error_islands(self):
        from scipy.ndimage import label
        x=np.linspace(-1,1,129);xx,yy=np.meshgrid(x,x)
        error=np.exp(-((xx-yy)/.15)**2)*(1+.4*np.cos(xx*90)*np.cos(yy*90))
        mask=choose_domain(error,64,np.ones((32,32),bool),1000,graded=True)
        self.assertGreater(mask.sum(),0);self.assertLess(mask.sum(),4096);self.assertLessEqual(inside_basis(mask).sum(),1000)
        self.assertLessEqual(label(mask)[1],2)

    def test_refinement_avoids_already_dense_neighborhood(self):
        density=np.ones((64,64));density[:,:32]=8
        x=np.linspace(-1,1,129);xx,yy=np.meshgrid(x,x);error=np.exp(-((abs(xx)-.5)**2+yy**2)/.08)
        mask=choose_domain(error,64,np.ones((32,32),bool),1000,graded=True,density=density)
        self.assertGreater(mask[:,32:].sum(),mask[:,:32].sum())
        self.assertLessEqual(inside_basis(mask).sum(),1000)

    def test_saved_models_restore_stably(self):
        x=np.linspace(-16000,16000,257);u=cardinal(256,x)
        with sqlite3.connect('build/local-spline-v7/local.sqlite3') as db:rows=db.execute('SELECT * FROM surfaces').fetchall()
        self.assertEqual(len(rows),24)
        for region,keep,raw,sb,vb in rows:
            info=json.loads(raw);t,labels=unpack(sb);v=np.frombuffer(zlib.decompress(vb),'<f4');self.assertEqual(len(labels),len(v));
            if keep<=.125:
                self.assertEqual(info['allocation'],'balanced');self.assertGreater(info['commonFilterMeters'],0);self.assertTrue(np.isfinite(info['commonFilterRmse']))
            if keep==.0625:self.assertLessEqual(max(n for n,index in labels),128)
            self.assertLessEqual(len(v),info['budget']);self.assertEqual(len(sb)+len(vb),info['cumulativeBytes']);self.assertTrue(np.isfinite(t.data).all());self.assertTrue((t.data>=0).all())
            dense=(t@v).reshape(261,261).astype('<f4');h=u@dense@u.T
            with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as source:blob=source.execute('SELECT coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()[0]
            reference=np.frombuffer(zlib.decompress(blob),'<f4').reshape(261,261);rmse=float(np.sqrt(np.mean((h-u@reference@u.T)**2)))
            self.assertAlmostEqual(rmse,info['heightRmse'],places=5)
if __name__=='__main__':unittest.main()
