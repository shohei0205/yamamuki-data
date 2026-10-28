"""自由 XY 再配置と点数の比較画面を作る。"""
import re
from pathlib import Path
root=Path('build/mls-generated-preview')
s=(root/'dem-six/index.html').read_text(encoding='utf-8')
s=s.replace('6点前後の局所MLS比較','狭いMLS・XY配置と点数の比較').replace('50 m・8,192点（試験）','50 m・同じLPF目標')
options='<option value="six-trained">従来配置・8,192点</option><option value="six-xy" selected>XY再配置・8,192点</option><option value="six-more">XY再配置・16,384点</option><option value="lpf-refit">参考：広い補間・8,192点</option>'
s=re.sub(r'<select id="variantSelect">.*?</select>','<select id="variantSelect">'+options+'</select>',s)
s=s.replace("'six-point','six-trained'].includes", "'six-point','six-trained','six-xy','six-more'].includes")
s=s.replace("||'lpf-refit'", "||'six-xy'")
s=s.replace('二次MLS ／ 6点前後の狭い補間範囲の試験','二次MLS ／ XY配置・点数の比較')
s=s.replace('LPF幅は制御点の平均間隔の1/4。','比較するLPF幅は全方式で88.4 mに固定。')
from mls_control_wire import inject
s=inject(s)
dest=root/'dem-xy/index.html';dest.parent.mkdir(exist_ok=True);dest.write_text(s,encoding='utf-8',newline='\n')
