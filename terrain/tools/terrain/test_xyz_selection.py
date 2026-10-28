"""全国のカメラ選択で、予算・包含関係・隣接差と固定レベルを確かめる。"""
import math
import unittest
from pathlib import Path
from xyz_selection import TileCatalog


class SelectionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog=TileCatalog(Path('build/terrain-national-xyz-20261024/terrain.sqlite3').resolve())

    def camera(self, distance=7000, maximum=14, mode='auto', divisions=1):
        latitude,longitude=35.3606,138.7274
        origin=[(longitude+180)/360*268435456,(.5-math.log(math.tan(math.pi/4+math.radians(latitude)/2))/(2*math.pi))*268435456]
        return dict(origin=origin,target=[0,1800,0],distance=distance,yaw=0,pitch=.85,viewport=[1400,1000],maximum=maximum,mode=mode,divisions=divisions)

    def validate(self, result):
        leaves=set(map(tuple,result['tiles']))
        self.assertLessEqual(len(leaves),result['tileBudget'])
        for z,x,y in leaves:
            self.assertLessEqual(z,result['maximum'])
            for ancestor in range(6,z):
                self.assertNotIn((ancestor,x//2**(z-ancestor),y//2**(z-ancestor)),leaves)
        for a in leaves:
            sa=2**(14-a[0]); ax,ay=a[1]*sa,a[2]*sa
            for b in leaves:
                sb=2**(14-b[0]); bx,by=b[1]*sb,b[2]*sb
                adjacent=((ax+sa==bx or bx+sb==ax) and max(ay,by)<min(ay+sa,by+sb)) or ((ay+sa==by or by+sb==ay) and max(ax,bx)<min(ax+sa,bx+sb))
                if adjacent and result['mode']=='auto':self.assertLessEqual(abs(a[0]-b[0]),2)

    def test_near_and_far_and_high_maximum(self):
        for maximum in (11,12,13,14):
            near=self.catalog.select(self.camera(maximum=maximum))
            far=self.catalog.select(self.camera(3600000,maximum))
            self.validate(near);self.validate(far)
            self.assertGreaterEqual(max(map(int,near['levelCounts'])),max(map(int,far['levelCounts'])))
            self.assertLessEqual(near['sourceTriangles'],350000)

    def test_subdivision_does_not_change_data_selection(self):
        camera=self.camera(3500,divisions=1);camera['pitch']=.07
        baseline=self.catalog.select(camera)
        self.assertGreater(max(map(int,baseline['levelCounts'])),6)
        for divisions in (2,4,8):
            camera['divisions']=divisions
            self.assertEqual(self.catalog.select(camera)['tiles'],baseline['tiles'])

    def test_fixed_keeps_requested_level(self):
        for z in range(6,15):
            camera=self.camera(min(7000,40075016.68557849/2**z*6),z,'fixed',2)
            for _ in range(20):
                result=self.catalog.select(camera)
                if 'retryDistance' not in result:break
                camera['distance']=result['retryDistance']
            self.assertNotIn('retryDistance',result)
            self.assertTrue(result['tiles'])
            self.assertEqual(set(map(int,result['levelCounts'])),{z})
            self.validate(result)

    def test_tilt_and_rotation_are_bounded(self):
        for pitch in (.03,.4,.85,1.55):
            for yaw in (0,1,2,3):
                camera=self.camera(divisions=4);camera.update(pitch=pitch,yaw=yaw)
                result=self.catalog.select(camera)
                self.validate(result)
                self.assertLessEqual(result['sourceTriangles'],350000)


if __name__=='__main__':unittest.main()
