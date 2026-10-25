"""z6 が空白を補完し、表示済みの地形を上書きしないことを実画像で確認する。"""
import argparse
import json
from pathlib import Path
import subprocess
from PIL import Image, ImageChops, ImageFilter, ImageStat

parser=argparse.ArgumentParser()
parser.add_argument('--source',type=Path)
parser.add_argument('--binary',type=Path)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--verify-only',action='store_true')
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
if not args.verify_only:
    assert args.source and args.binary,'--source と --binary が必要です'
    assert not (args.output/'with-z6.png').exists(),'既存画像を上書きしません'
    result=subprocess.run([str(args.binary.resolve()),str(args.source.resolve()),'--region','fuji','--divisions','4','--underlay-test-output',str(args.output.resolve())],capture_output=True,timeout=30)
    (args.output/'stdout.log').write_bytes(result.stdout)
    (args.output/'stderr.log').write_bytes(result.stderr)
    assert result.returncode==0,result.stderr.decode('utf-8',errors='replace')
before=Image.open(args.output/'without-z6.png').convert('RGB')
after=Image.open(args.output/'with-z6.png').convert('RGB')
assert before.size==after.size
# 設定パネルと右上のFPS表示は比較から除外する。
box=(0,max(240,before.height//3),before.width,before.height)
before=before.crop(box);after=after.crop(box)
sky=Image.new('RGB',before.size,(230,241,250))
def mask(image):
    red,green,blue=ImageChops.difference(image,sky).split()
    return ImageChops.lighter(ImageChops.lighter(red,green),blue).point(lambda p:255 if p>4 else 0)
a=mask(before);b=mask(after)
# 画像補間が混ざる輪郭の数画素は除き、地形の内部を比較する。
inside=a.filter(ImageFilter.MinFilter(5))
difference=ImageChops.difference(before,after)
mean=ImageStat.Stat(difference,inside).mean
maximum=max(v[1] for v in ImageChops.multiply(difference,Image.merge('RGB',(inside,inside,inside))).getextrema())
report={'foreground_pixels_before':a.histogram()[255],'terrain_pixels_after':b.histogram()[255],'foreground_mean_difference':mean,'foreground_max_difference':maximum}
assert report['terrain_pixels_after']>report['foreground_pixels_before']+1000,report
assert max(mean)<0.01 and maximum<=1,report
report['passed']=True
(args.output/'verification.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
print(json.dumps(report))
