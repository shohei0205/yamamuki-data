"""太陽の指定値と、既定光源のキャッシュを確認する。"""
import unittest
from national_sun import angles, status


class NationalSunTest(unittest.TestCase):
    def test_angle_limits(self):
        self.assertEqual(angles({'azimuth': 360, 'altitude': 90}), (0, 90))
        for values in [{'azimuth': -1, 'altitude': 30}, {'azimuth': 0, 'altitude': 91},
                       {'azimuth': True, 'altitude': 45}, {'azimuth': 1.5, 'altitude': 30}]:
            with self.assertRaises(ValueError):
                angles(values)

    def test_default_image_is_reused(self):
        result = status({'azimuth': 315, 'altitude': 45})
        self.assertEqual(result['state'], 'complete')
        self.assertEqual(result['imageURL'], '/national-shading-data/shade.png')


if __name__ == '__main__':
    unittest.main()
