"""3地域・全誤差段階の元DEM比較画面を既存の同一描画実装から作る。"""
import re
from pathlib import Path

ROOT = Path('build/mls-generated-preview')
html = (ROOT/'yari-dem/index.html').read_text(encoding='utf-8')
html = re.sub(r'<p id="region-navigation">.*?</p>', '', html)
html = html.replace('<title>槍ヶ岳・元PNG DEMとの比較</title>', '<title>3地域・元DEM基準のMLS品質比較</title>')
html = html.replace('槍ヶ岳 ／ 元PNG DEM基準・独立と残差の比較', '富士山・槍ヶ岳・筑波山 ／ 元DEM基準の曲面比較')
options = ''.join(f'<option value="{e}-independent">{e} m・独立</option>'
                  for e in ('12.5', '25', '50', '100', '200', '400', '800', '1600', '3200'))
options += '<option value="12.5-residual">12.5 m・残差曲面（25 m独立が親）</option>'
html = re.sub(r'<select id="modelSelect">.*?</select>', '<select id="modelSelect">'+options+'</select>', html)
html = html.replace('<label>方式 ', '<label>地域 <select id="regionSelect"><option value="fuji">富士山</option><option value="yari">槍ヶ岳</option><option value="tsukuba">筑波山</option></select></label>　<label>方式 ')
html = html.replace('const xyBuffers=[],fieldBuffers=[];let info,loadTicket=0;',
                    'const xyBuffers=[],fieldBuffers=[];let info,loadTicket=0,focus=2300;')
html = html.replace('uniform float yaw,tilt,span,aspect;', 'uniform float yaw,tilt,span,aspect,focus;')
html = html.replace('field.x-2300.', 'field.x-focus')
html = html.replace('{yaw,tilt,span,aspect:', '{yaw,tilt,span,focus,aspect:')
html = html.replace("kind=document.getElementById('modelSelect').value;", "kind=document.getElementById('modelSelect').value,region=document.getElementById('regionSelect').value;")
html = html.replace("fetch(kind+'.json')", "fetch(region+'/'+kind+'.json')")
html = html.replace("fetch(name+'-'+range+'.bin')", "fetch(region+'/'+name+'-'+range+'.bin')")
html = html.replace('info=nextInfo;', "info=nextInfo;focus=region==='tsukuba'?300:2300;")
html = html.replace("history.replaceState(null,'','?quality='+kind);", "history.replaceState(null,'','?region='+region+'&quality='+kind);")
start = html.index("const initial=new URLSearchParams(location.search).get('quality');")
end = html.index('loadRange().catch', start)
html = html[:start]+'''const params=new URLSearchParams(location.search),initial=params.get('quality'),initialRegion=params.get('region');
if([...document.getElementById('modelSelect').options].some(o=>o.value===initial))document.getElementById('modelSelect').value=initial;
if(['fuji','yari','tsukuba'].includes(initialRegion))document.getElementById('regionSelect').value=initialRegion;
document.getElementById('regionSelect').onchange=()=>loadRange(true).catch(e=>document.getElementById('status').textContent=e.message);
''' + html[end:]
html = html.replace('../yari-levels/', '../yari-dem/').replace('旧Bスプライン基準の比較', '従来の槍ヶ岳3方式比較')
dest = ROOT/'dem-regions/index.html'
dest.parent.mkdir(exist_ok=True)
dest.write_text(html, encoding='utf-8', newline='\n')
old = ROOT/'yari-dem/index.html'
source = old.read_text(encoding='utf-8')
if 'id="region-navigation"' not in source:
    source = source.replace('<header>', '<header><p id="region-navigation"><a href="../dem-regions/">富士山・槍ヶ岳・筑波山の全品質比較へ</a></p>')
    old.write_text(source, encoding='utf-8', newline='\n')
