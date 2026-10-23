import tempfile,unittest,zlib,struct
from pathlib import Path
from unittest.mock import patch
import numpy as np
from q_mesh_settings import definitions,nearby_density_ratio
from national_q_mesh import catalog_keys,generate,estimated_job_bytes,admission_allowed

class NationalQTest(unittest.TestCase):
    def test_level_and_density_definitions(self):
        levels=definitions();self.assertEqual([l['level'] for l in levels],list(range(4,13)))
        self.assertEqual([l['maxErrorMeters'] for l in levels],[12.5,25,50,100,200,400,800,1600,3200])
        for l in levels:
            self.assertEqual(l['densityRadiusMeters'],l['widthMeters']/2)
            self.assertAlmostEqual(nearby_density_ratio(4096,l),1)
            self.assertAlmostEqual(nearby_density_ratio(2048,l),.5)

    def test_parallel_admission_counts_all_active_jobs(self):
        small=estimated_job_bytes(definitions()[0]);large=estimated_job_bytes(definitions()[-1])
        self.assertTrue(admission_allowed([small]*3,small,small*4))
        self.assertFalse(admission_allowed([small]*4,small,small*4))
        self.assertFalse(admission_allowed([large],small,small*4))
        self.assertTrue(admission_allowed([],large,small*4))

    def test_catalog_is_aligned_across_negative_coordinates(self):
        self.assertEqual(len(catalog_keys([(-1,-1)],2000)),16)
        self.assertEqual(catalog_keys([(-1,-1),(0,0)],16000),[(-1,-1),(0,0)])
        self.assertEqual(catalog_keys([(0,0),(1,1)],16000),[(0,0)])

    def test_output_contains_only_positions_faces_and_working_validity(self):
        spec=dict(definitions()[0],widthMeters=64,sampleSpacingMeters=1)
        p=np.array([[0,0,2],[64,0,2],[64,64,2],[0,64,2]],float);f=np.array([[0,1,2],[0,2,3]])
        with patch('national_q_mesh.sample_grid',return_value=(np.full((65,65),2,np.float32),np.ones((65,65),bool))),patch('national_q_mesh.base_mesh',return_value=(p,f)),patch('national_q_mesh.national.BINARY','unused',create=True):
            nv,nf,mesh,validity=generate((spec,0,0))
        raw=zlib.decompress(mesh);self.assertEqual(struct.unpack_from('<4sII',raw),(b'YQB1',4,2));self.assertEqual(len(raw),12+4*12+2*12)
        self.assertEqual((nv,nf),(4,2));self.assertTrue(np.unpackbits(np.frombuffer(zlib.decompress(validity),np.uint8),bitorder='little')[:65*65].all())

if __name__=='__main__':unittest.main()
