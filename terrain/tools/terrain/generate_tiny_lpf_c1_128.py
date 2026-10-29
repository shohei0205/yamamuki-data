"""3地域の12.5m版で小幅LPFを比較。既存の配置・点数を保持。"""
import generate_continuous_c1_128 as generator
from tiny_lpf_dem import sample,STEP,filtered_tile

def run():
 for name,sigma in [('lpf05',.5),('lpf1',1.)]:
  generator.OUT=generator.ROOT/f'build/natural-raw-c1-128-{name}-v1'
  generator.sample=lambda q,sigma=sigma:sample(q,sigma)
  generator.DEM_METADATA=dict(lpf=True,lpfSigmaPixels=sigma,lpfSigmaMercatorMeters=sigma*STEP,sourceInterpolation='bilinear-after-nodata-normalized-gaussian',sameXY=True)
  generator.run();filtered_tile.cache_clear()
if __name__=='__main__':run()
