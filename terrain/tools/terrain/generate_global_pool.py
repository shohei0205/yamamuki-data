"""保存タイルの余剰を共通プールへ戻し、最新誤差順に一頂点ずつ再配分する。"""
import argparse, heapq, json, sqlite3, subprocess, time
from contextlib import closing
import numpy as np
from generate_shared_quality_budgets import cached_prepare
from refine_shared_quality import ROOT, compare, save, angles
from compare_shared_allocation import residual
from uncut_mesh_topology import tile_topology, update_metrics
from verify_shared_allocation import verify


def run(base=64, window_limit=864):
    mode='global-pool128-unlimited' if window_limit==0 else 'global-pool128';path=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
    if path.exists():raise ValueError('既存の比較出力は上書きしません')
    binary=ROOT/'build/terrain-rust-local-budget-target/release/terrain-adaptive-tin.exe'
    compare.initialize();regions=[];clients=[];logs=[];meshes=[];reports={};started=time.time()
    progress=ROOT/f'build/terrain-{mode}.progress.json'
    def request(client,command):
        client.stdin.write(command+'\n');client.stdin.flush();line=client.stdout.readline()
        if not line:raise RuntimeError('生成クライアントが終了しました: '+str(client.poll()))
        return json.loads(line)
    try:
        with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as source,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as template:
            manifest=json.loads(source.execute('select value from metadata where key="manifest"').fetchone()[0])
            templates=json.loads(template.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions']
            total=base*256*len(manifest['reliefRegions']);ready=[];outputs=[]
            for area in manifest['reliefRegions']:
                name=area['id'];r=cached_prepare(next(v for v in templates if v['id']==name),source);regions.append(r)
                output=ROOT/f'build/terrain-{mode}-{name}.json';outputs.append(output)
                log=open(ROOT/f'build/terrain-{mode}-{name}.err','w',encoding='utf-8');logs.append(log)
                command=[str(binary),'--input',str(r['folder']/'height.png'),'--output',str(output),'--scale','.25','--offset',str(r['height_offset']),'--error-limits',str(r['folder']/'balanced.png'),'--tile-pixels','256','--tile-base',str(base),'--tile-global-pool','--tile-window-limit',str(window_limit),'--max-vertices',str(total)]
                client=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,encoding='utf-8');clients.append(client)
                row=json.loads(client.stdout.readline());assert row['ready'];ready.append(row)
                print('初期配分',name,sum(row['allocation']['counts']),flush=True)
            initial=[sum(v['allocation']['counts']) for v in ready];pool=total-sum(initial);initial_pool=pool
            assert pool==sum(sum(v['allocation']['donated']) for v in ready)
            heap=[];steps=[0]*len(clients)
            for i,v in enumerate(ready):
                if v['priority']>1:heapq.heappush(heap,(-v['priority'],i,v['sample']))
            trace=[]
            while pool and heap:
                priority,i,sample=heapq.heappop(heap);result=request(clients[i],'step')
                assert result['inserted'];pool-=1;steps[i]+=1
                if result['priority']>1:heapq.heappush(heap,(-result['priority'],i,result['sample']))
                if (initial_pool-pool)%256==0:
                    item=dict(allocated=initial_pool-pool,remaining=pool,counts=[a+b for a,b in zip(initial,steps)],priority=-priority,elapsedSeconds=round(time.time()-started,1))
                    trace.append(item);progress.write_text(json.dumps(item),encoding='utf-8');print('共通配分',item,flush=True)
            for i,(area,r,client,output) in enumerate(zip(manifest['reliefRegions'],regions,clients,outputs)):
                response=request(client,'save');assert window_limit==0 or response['maximumWindowLoad']<=window_limit
                data=json.loads(output.read_text(encoding='utf-8'));p=np.asarray(data['vertices'],float);f=np.asarray(data['triangles'],np.uint32);a=data['tileAllocation']
                assert len(p)==initial[i]+steps[i]==sum(a['counts'])==sum(a['quotas'])
                assert all(c==q for c,q in zip(a['counts'],a['quotas']))
                delta=residual(p,f,r['grid']);v=delta[r['valid']];ratio=float(np.max(np.abs(v)/r['shape_limits'][r['valid']]));angle=angles(p,f)
                mesh=dict(p=p,f=f,maximum=float(np.abs(v).max()),rmse=float(np.sqrt(np.mean(v*v))),weightedMaximum=ratio,score=ratio);meshes.append(mesh)
                reports[area['id']]=dict(allocation=a,sourceVertices=len(p),initialVertices=initial[i],addedFromPool=steps[i],maximumWindowLoad=response['maximumWindowLoad'],minimumAngle=float(angle.min()),under10=int((angle<10).sum()),beforeUnder10=int((angle<10).sum()),rmse=mesh['rmse'],maximum=mesh['maximum'])
                print('最終配分',area['id'],len(p),'3x3最大',response['maximumWindowLoad'],flush=True)
            assert sum(len(v['p']) for v in meshes)+pool==total
            save('balanced',regions,meshes,manifest,total,trace,variant=mode)
        with closing(sqlite3.connect(path)) as db,db:
            m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);topology=[]
            for area,mesh in zip(m['reliefRegions'],meshes):
                keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);records={tuple(v) for v in keys};world=mesh['p'].copy();world[:,:2]*=31.25;ranks=np.arange(len(world))/max(1,len(world)-1)
                for (x,y),body,count in tile_topology(world,mesh['f'],ranks=ranks):
                    key=(3,ox+x,oy+y)
                    if key in records:db.execute('insert or replace into topology values(?,?,?,?)',(*key,body))
                topology.append(dict(region=area['id'],source='direct-parent-tin',facesBefore=len(mesh['f']),facesAfter=len(mesh['f'])))
            a=m['sharedAllocation'];a['topologyReports']=topology
            a['localAllocation']=dict(policy='global-pool',baseVerticesPerTile=base,tileWidthMeters=8000,windowSize=3,windowLimit=window_limit,sharing=True,selection='current-normalized-error',scope='three-disconnected-regions-common-pool',outsideTileReserve=base)
            a['globalPool']=dict(totalBudget=total,initialSurplus=initial_pool,allocatedSurplus=initial_pool-pool,unusedPool=pool)
            a['meshQuality']=dict(regions=reports);a['generationComplete']=True;update_metrics(db,m)
        result=verify(mode);report=dict(mode=mode,sourceVertices=result['totalSource'],budget=total,unusedPool=pool,tiles=result['tiles'],sharedEdges=result['sharedEdges'],regions=reports)
        (ROOT/f'build/terrain-{mode}-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
        progress.write_text(json.dumps(dict(complete=True,sourceVertices=result['totalSource'],unusedPool=pool,elapsedSeconds=time.time()-started)),encoding='utf-8')
        print('生成・保存検証完了',mode,result['totalSource'],'未使用',pool,flush=True)
    finally:
        for client in clients:
            if client.poll() is None:
                client.stdin.close()
                try:client.wait(timeout=10)
                except subprocess.TimeoutExpired:client.kill();client.wait()
        for log in logs:log.close()


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--base',type=int,default=64);parser.add_argument('--window-limit',type=int,default=864);args=parser.parse_args();run(args.base,args.window_limit)
