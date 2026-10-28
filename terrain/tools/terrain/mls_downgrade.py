"""高品位MLSのXYから重要度の低い点を削り、粗い品質を作る。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,json,sqlite3,time,zlib,hashlib
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import requirements,peak_samples,fit_model

def removal_order(original,points,tolerance):
    support=LocalSupport(points,pre_smooth=False,floor_factor=.5)
    a=operator(points,points,support,curvature=1e-5);h=reference_values(original,points);leverage=a.diagonal()
    score=abs(h-a@h)/np.maximum(1-leverage,.05);score/=np.where(np.linalg.norm(points,axis=1)<=2000,tolerance*.5,tolerance)
    protected=np.all(abs(points)==SPAN/2,axis=1);score[protected]=np.inf
    return np.argsort(score,kind='stable'),support(points),protected

def choose_removals(points,order,radii,protected,wanted):
    tree=cKDTree(points);blocked=protected.copy();chosen=[]
    for i in order:
        if blocked[i]:continue
        chosen.append(int(i));blocked[tree.query_ball_point(points[i],float(radii[i])*.5)]=True
        if len(chosen)>=wanted:break
    return np.asarray(chosen,int)

def generate(a):
    if a.minimum<4 or a.batch_min<1:raise ValueError('点数とバッチ下限は正で指定してください')
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'quality.sqlite3'
    if path.exists():raise ValueError('既存の結果を残すため新しい出力先を指定してください')
    axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis)
    audit=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
    with sqlite3.connect(a.seed/'quality.sqlite3') as seed,sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE levels(region TEXT,target_error REAL,info TEXT,points BLOB,PRIMARY KEY(region,target_error)) WITHOUT ROWID');report=[];history=[]
        for region in a.regions:
            seed_generation_seconds=sum(json.loads(row[0])['seconds'] for row in seed.execute('SELECT info FROM levels WHERE region=?',(region,)))
            errors=sorted(set(a.errors));raw,blob=seed.execute('SELECT info,points FROM levels WHERE region=? AND target_error=?',(region,errors[0])).fetchone();seed_info=json.loads(raw)
            decoded=np.frombuffer(zlib.decompress(blob),'<f4');data=decoded[1:].reshape(-1,3);seed_points=data[:,:2].copy();ids=np.arange(len(data));z=data[:,2].copy()
            b=source.execute('SELECT coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()[0];original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,audit)
            support=LocalSupport(seed_points,pre_smooth=False,floor_factor=.5);initial_error=operator(seed_points,audit,support,curvature=1e-5)@z-truth
            if not requirements(initial_error,audit,errors[0])['converged']:raise ValueError('最高品質の入力モデルが誤差条件を満たしていません')
            checkpoint_target=None;checkpoint_parent=a.checkpoint.parent/'quality.sqlite3' if a.checkpoint else None
            if a.checkpoint:
                with np.load(a.checkpoint,allow_pickle=False) as checkpoint:
                    if checkpoint['region'].item()==region:
                        if checkpoint['seedSha'].item()!=hashlib.sha256(seed_points.tobytes()).hexdigest():raise ValueError('チェックポイントの元点配置が一致しません')
                        checkpoint_target=float(checkpoint['targetError'])
            for tolerance in errors:
                if checkpoint_target is not None and errors[0]<tolerance<checkpoint_target and checkpoint_parent.exists():
                    with sqlite3.connect(checkpoint_parent) as parent_db:parent_raw,parent_blob=parent_db.execute('SELECT info,points FROM levels WHERE region=? AND target_error=?',(region,tolerance)).fetchone()
                    parent_ids=np.load(a.checkpoint.parent/f'lineage-{region}-E{tolerance:g}.npy',allow_pickle=False)
                    if not set(parent_ids).issubset(set(ids)):raise ValueError('チェックポイントの親レベルの包含関係が一致しません')
                    parent_data=np.frombuffer(zlib.decompress(parent_blob),'<f4')[1:].reshape(-1,3)
                    if not np.array_equal(parent_data[:,:2],seed_points[parent_ids]):raise ValueError('チェックポイントの親レベルのXYが一致しません')
                    ids=parent_ids.copy();z=parent_data[:,2].copy();support=LocalSupport(seed_points[ids],pre_smooth=False,floor_factor=.5);initial_error=operator(seed_points[ids],audit,support,curvature=1e-5)@z-truth
                    if not requirements(initial_error,audit,tolerance)['converged']:raise ValueError('チェックポイントの親レベルの誤差条件が未達です')
                    info=json.loads(parent_raw);info['copiedFromCheckpointParent']=True;db.execute('INSERT INTO levels VALUES(?,?,?,?)',(region,tolerance,json.dumps(info),parent_blob));db.commit();report.append(info);np.save(a.output/f'lineage-{region}-E{tolerance:g}.npy',ids,allow_pickle=False);continue
                started=time.monotonic();before=len(ids);trials=0;accepted=0;current_error=initial_error.copy();stop='seed-high-quality';resumed_controls=None;proposal=max(a.batch_min,int(len(ids)*.25))
                if a.checkpoint and tolerance!=errors[0]:
                    with np.load(a.checkpoint,allow_pickle=False) as checkpoint:
                        if checkpoint['region'].item()==region and float(checkpoint['targetError'])==tolerance:
                            if checkpoint['seedSha'].item()!=hashlib.sha256(seed_points.tobytes()).hexdigest():raise ValueError('チェックポイントの元点配置が一致しません')
                            restored_ids=checkpoint['ids'].copy()
                            if not set(restored_ids).issubset(set(ids)):raise ValueError('チェックポイントの点が現在の親レベルに含まれていません')
                            ids=restored_ids;z=checkpoint['z'].copy();proposal=int(checkpoint['proposal']);resumed_controls=len(ids);support=LocalSupport(seed_points[ids],pre_smooth=False,floor_factor=.5);current_error=operator(seed_points[ids],audit,support,curvature=1e-5)@z-truth
                            if not requirements(current_error,audit,tolerance)['converged']:raise ValueError('チェックポイントの誤差条件が未達です')
                if tolerance!=errors[0]:
                    while len(ids)>a.minimum:
                        points=seed_points[ids];order,radii,protected=removal_order(original,points,tolerance)
                        wanted=min(len(ids)-a.minimum,proposal);success=False
                        while wanted>=a.batch_min:
                            remove=choose_removals(points,order,radii,protected,wanted)
                            if not len(remove):stop='no-removable-points';break
                            keep=np.ones(len(ids),bool);keep[remove]=False;candidate_ids=ids[keep];candidate_points=seed_points[candidate_ids];trials+=1
                            try:
                                candidate_z,candidate_support,iterations=fit_model(original,candidate_points,tolerance)
                                error=operator(candidate_points,audit,candidate_support,curvature=1e-5)@candidate_z-truth;measurement=requirements(error,audit,tolerance)
                            except ValueError as ex:measurement=dict(converged=False,error=str(ex))
                            entry=dict(region=region,targetError=tolerance,fromControls=len(ids),trialControls=len(candidate_ids),removed=len(remove),accepted=bool(measurement['converged']),measurement=measurement);history.append(entry)
                            (a.output/'progress.json').write_text(json.dumps(entry,indent=2),encoding='utf-8');print(json.dumps(entry),flush=True)
                            if measurement['converged']:
                                ids=candidate_ids;z=candidate_z;support=candidate_support;current_error=error;accepted+=1;success=True;proposal=max(a.batch_min,min(int(len(ids)*.25),int(len(remove)*1.5)))
                                np.savez(a.output/'checkpoint.npz',ids=ids,z=z,targetError=np.array(tolerance),region=np.array(region),proposal=np.array(proposal),seedSha=np.array(hashlib.sha256(seed_points.tobytes()).hexdigest()))
                                break
                            if wanted<=a.batch_min or len(remove)<=a.batch_min:break
                            wanted=max(a.batch_min,min(wanted//2,len(remove)//2))
                        if not success:stop='no-acceptable-batch';break
                    else:stop='minimum-points'
                measurement=requirements(current_error,audit,tolerance)
                if not measurement['converged']:raise ValueError('保存候補が誤差条件を満たしていません')
                points=seed_points[ids];packed=zlib.compress(np.array([1.25],'<f4').tobytes()+np.column_stack((points,z)).astype('<f4').tobytes(),6);radii=support(audit[::16]);info=dict(seed_info)
                info.update(resumedFromControls=resumed_controls,seedGenerationSeconds=seed_generation_seconds,seedReused=True,peakError=float(abs((operator(points,np.array([[0.,0.]]),support,curvature=1e-5)@z)[0]-reference_values(original,np.array([[0.,0.]]))[0])),targetError=tolerance,targetPeakError=tolerance*.5,controls=len(ids),budget=len(ids),qualityMethod='downgrade',seedControls=len(seed_points),parentControls=before,removedFromParent=before-len(ids),qualityMode=True,**measurement,stopReason=stop,rmse=measurement['measuredRmse'],maxError=measurement['measuredMaxError'],storedBytes=len(packed),rawBytes=4+len(ids)*12,searchTrials=trials,acceptedBatches=accepted,minimumPoints=a.minimum,batchMinimum=a.batch_min,radiusMeters=float(np.median(radii)),radiusMin=float(radii.min()),radiusMax=float(radii.max()),spacingMeters=SPAN/np.sqrt(len(ids)),seconds=time.monotonic()-started)
                db.execute('INSERT INTO levels VALUES(?,?,?,?)',(region,tolerance,json.dumps(info),packed));db.commit();report.append(info);initial_error=current_error
                np.save(a.output/f'lineage-{region}-E{tolerance:g}.npy',ids.astype('<u4'),allow_pickle=False)
                (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8');(a.output/'history.json').write_text(json.dumps(history,indent=2),encoding='utf-8');print('LEVEL',json.dumps(info),flush=True)
    (a.output/'progress.json').write_text(json.dumps(dict(stage='complete',levels=len(report)),indent=2),encoding='utf-8')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--seed',type=Path,default=Path('build/mls-error-quality-v7'));p.add_argument('--output',type=Path,default=Path('build/mls-downgrade-v2'));p.add_argument('--regions',nargs='+',default=['fuji']);p.add_argument('--errors',nargs='+',type=float,default=[25,50,100,200]);p.add_argument('--minimum',type=int,default=256);p.add_argument('--batch-min',type=int,default=16);p.add_argument('--checkpoint',type=Path);a=p.parse_args();generate(a)
