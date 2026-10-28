"""6点程度の狭いMLSと広いMLSを同一視点で比較する専用画面。"""
import re
from pathlib import Path

root = Path('build/mls-generated-preview')
html = (root/'dem-lpf/index.html').read_text(encoding='utf-8')
html = html.replace('自由配置MLS ／ LPF目標・曲率抑制の比較', '二次MLS ／ 6点前後の狭い補間範囲の試験')
html = html.replace('3地域・LPF目標と波打ち抑制の比較', '6点前後の局所MLS比較')
html = re.sub(r'<select id="modelSelect">.*?</select>', '<select id="modelSelect"><option value="50-independent">50 m・8,192点（試験）</option></select>', html)
options = '<option value="lpf-refit">従来・広い範囲（約53点）</option><option value="six-point">狭い範囲・初期版（中央値7点）</option><option value="six-trained" selected>狭い範囲・傾きも適合（中央値8点）</option>'
html = re.sub(r'<select id="variantSelect">.*?</select>', '<select id="variantSelect">'+options+'</select>', html)
html = html.replace("'lpf-smooth','lpf-refit'].includes(params.get('variant'))", "'lpf-smooth','lpf-refit','six-point','six-trained'].includes(params.get('variant'))")
html = html.replace('window.levelInfo=info;', "if(info.neighborCountMedian!==undefined)document.getElementById('status').textContent+=' 参照点数：最小'+info.neighborCountMin+'／中央値'+info.neighborCountMedian+'／最大'+info.neighborCountMax+'。';window.levelInfo=info;")
html = html.replace('LPF幅は制御点の平均間隔の1/4。', '6点を下限に、点の出入りを連続にする重なりを確保。生成と表示は同じ半径・二次MLS。LPF幅は制御点の平均間隔の1/4。')
html = html.replace('../dem-regions/', '../dem-regions/')
html = html.replace('<a href="../dem-regions/">従来方式・全地域全段階の比較</a>', '<a href="../dem-lpf/?variant=lpf-refit">全品質・従来方式の比較へ</a>')
dest = root/'dem-six/index.html'; dest.parent.mkdir(exist_ok=True)
dest.write_text(html, encoding='utf-8', newline='\n')
