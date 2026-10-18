import sys,time,json,gzip,struct,unittest
from pathlib import Path
import numpy as np
sys.path.insert(0,str(Path(__file__).parent))
from terrain_transport import encode_payload
class TransportTests(unittest.TestCase):
    def test_arrays_roundtrip(self):
        source={'mesh':[[1.125,-2,3],[4,5,6]],'indices':[0,1,0],'landIndices':[], 'overlays':{'layers':{'water':{'lines':[1,2,3,4,5,6],'triangles':[]}}},'name':'地形'}
        packed=encode_payload(source);magic,length=struct.unpack_from('<4sI',packed)
        self.assertEqual(magic,b'YTB1');self.assertEqual(length%4,0)
        header=json.loads(packed[8:8+length]);offset=8+length
        for item in header['arrays']:
            values=np.frombuffer(packed,dtype=item['type'],count=item['count'],offset=offset);offset+=item['count']*4
            expected=source
            for key in item['path']:expected=expected[key]
            np.testing.assert_array_equal(values,np.asarray(expected).ravel())
        self.assertEqual(offset,len(packed));self.assertEqual(source['indices'],[0,1,0]);self.assertEqual(header['metadata']['name'],'地形')
if __name__=='__main__':unittest.main()
