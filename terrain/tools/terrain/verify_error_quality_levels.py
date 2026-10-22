"""品質6段階の予算・誤差・頂点数と、混在タイルの接続を検証する。"""
import json,sqlite3
from contextlib import closing
from generate_error_quality_levels import ROOT,ERRORS,mode,collect
from verify_shared_allocation import verify
from verify_error_detail20 import verify_mixed

def main(errors=ERRORS):
    all_levels=collect();totals=[r['sourceVertices'] for r in all_levels];assert all(a>b for a,b in zip(totals,totals[1:]));levels=[r for r in all_levels if r['errorMeters'] in errors];assert [v['errorMeters'] for v in levels]==errors
    previous=float('inf');report=[]
    for row in levels:
        error=row['errorMeters'];policy=row['policy'];assert policy['additionalBudget']==policy['basicVertices']//5;assert policy['additionalUsed']<=policy['additionalBudget'];assert row['sourceVertices']<previous;previous=row['sourceVertices']
        assert sum(v['sourceVertices'] for v in row['regions'].values())==row['sourceVertices']
        for r in row['regions'].values():assert r['maximum']<=error+1e-5 and r['minimumAngle']>=10.-1e-6
        uniform=verify(row['mode']);mixed=verify_mixed(row['mode'])
        with closing(sqlite3.connect(ROOT/f"build/terrain-shared-allocation-{row['mode']}-mixed.sqlite3")) as db:
            m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);assert m['levels'][0]['maxErrorMeters']==error;assert m['sharedAllocation']['sourceVertices']==row['sourceVertices'];assert m['sharedAllocation']['errorDetailPolicy']==policy
        report.append(dict(quality=row['quality'],errorMeters=error,sourceVertices=row['sourceVertices'],uniformTiles=uniform['tiles'],mixed=mixed));print('品質検証',error,row['sourceVertices'],flush=True)
    path=ROOT/'build/terrain-error-quality-verification.json';old=json.loads(path.read_text()) if path.exists() else [];merged={r['quality']:r for r in old};merged.update({r['quality']:r for r in report});path.write_text(json.dumps([merged[k] for k in sorted(merged)],ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--errors',type=int,nargs='+',default=ERRORS);args=parser.parse_args();main(sorted(args.errors))
