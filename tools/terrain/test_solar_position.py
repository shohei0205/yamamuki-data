import unittest
from solar_position import ephemeris_jst
from national_sun import specification
import math
import tempfile
from pathlib import Path
from unittest.mock import patch,Mock
import national_sun

class SolarPositionTest(unittest.TestCase):
    def test_jst_and_reference(self):
        d,h=ephemeris_jst('2003-10-18T04:30')
        lat=math.radians(39.742476);hour=h+math.radians(-105.1786)
        east=-math.cos(d)*math.sin(hour)
        north=math.sin(d)*math.cos(lat)-math.cos(d)*math.cos(hour)*math.sin(lat)
        az=(math.degrees(math.atan2(east,north))+360)%360
        self.assertAlmostEqual(az,194.34024,delta=.2)
    def test_cache_keys_and_invalid_dates(self):
        a,_=specification({'datetime':'2026-03-20T06:00'})
        b,_=specification({'datetime':'2026-03-20T07:00'})
        self.assertEqual(a['key'],'t20260320T0600')
        self.assertNotEqual(a['key'],b['key'])
        for v in ['2026-02-30T12:00','bad','2026-03-20T06:00+09:00',None,'1800-01-01T00:00']:
            with self.assertRaises((ValueError,TypeError)):
                ephemeris_jst(v)

    def test_dated_generator_command(self):
        with tempfile.TemporaryDirectory(dir='build') as folder:
            root=Path(folder)
            (root/'build/rust-tools/llvm-mingw/compiler').mkdir(parents=True)
            process=Mock();process.pid=123;process.poll.return_value=None
            options={'datetime':'2026-03-20T06:00'}
            with patch.object(national_sun,'ROOT',root),patch.object(national_sun,'OUTPUT',root/'output'),patch.object(national_sun,'JOBS',{}),patch.object(national_sun.subprocess,'Popen',return_value=process) as launch,patch.object(national_sun,'status',side_effect=[{'state':'missing','key':'t20260320T0600'},{'state':'running'}]):
                result=national_sun.start(options)
                self.assertEqual(result['state'],'running')
                command=launch.call_args.args[0]
                self.assertIn('--solar-datetime',command)
                self.assertIn('2026-03-20T06:00',command)
                self.assertTrue(any(v.startswith('--solar-hour-angle=') for v in command))
                self.assertNotIn('--sun-azimuth',command)

if __name__=='__main__':unittest.main()
