//! 粗いレベルの追加処理を確定してから子を生成する。基本・追加・差分を別SQLiteに保存する。
use super::*;
use std::collections::{BTreeMap,HashSet};
type Point=[i64;2];
type Face=[usize;3];
struct Geometry {p:Vec<Point>,z:Vec<f64>,f:Vec<Face>}
const SCHEMA:&str="CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);
CREATE TABLE IF NOT EXISTS tiles(q INTEGER,x INTEGER,y INTEGER,parent_q INTEGER,parent_x INTEGER,parent_y INTEGER,vertices INTEGER,triangles INTEGER,inherited INTEGER,parent_refs BLOB,new_vertices BLOB,indices BLOB,border_indices BLOB,PRIMARY KEY(q,x,y));";
fn decode(body:&[u8],n:usize)->Result<Geometry> {
    let mut raw=Vec::new();ZlibDecoder::new(body).read_to_end(&mut raw)?;
    let (points,faces)=q_pool_bench::decode(&raw,n)?;
    let z=(0..points.len()).map(|i|f32::from_le_bytes(raw[20+i*12..24+i*12].try_into().unwrap()) as f64).collect();
    Ok(Geometry{p:points.iter().map(|p|[p[0] as i64,p[1] as i64]).collect(),z,f:faces})
}
fn body(g:&Geometry)->Result<Vec<u8>> {
    ensure!(g.p.len()==g.z.len()&&g.p.len()<=65535,"階層形状の頂点数が不正です");
    let mut raw=b"YQB1".to_vec();raw.extend((g.p.len() as u32).to_le_bytes());raw.extend((g.f.len() as u32).to_le_bytes());
    for (p,z) in g.p.iter().zip(&g.z) {for v in [p[0] as f64*SPACING,p[1] as f64*SPACING,*z] {raw.extend((v as f32).to_le_bytes());}}
    for f in &g.f {for &i in f {raw.extend((i as u32).to_le_bytes());}}zip(&raw)
}
fn finished(base:&Connection,pool:Option<&Connection>,j:Job)->Result<Geometry> {
    let extra=pool.map(|db|db.query_row("SELECT mesh FROM additional WHERE q=?1 AND x=?2 AND y=?3",params![j.q,j.x,j.y],|r|r.get::<_,Vec<u8>>(0)).optional()).transpose()?.flatten();
    let blob=if let Some(b)=extra {b}else{base.query_row("SELECT mesh FROM basic WHERE q=?1 AND x=?2 AND y=?3",params![j.q,j.x,j.y],|r|r.get(0))?};decode(&blob,j.n())
}
fn stopped(args:&Args)->bool {args.stop_file.as_ref().is_some_and(|p|p.exists())}
fn status(args:&Args,stage:&str,q:i64,done:usize,total:usize,start:Instant)->Result<()> {
    fs::write(args.output.with_extension("progress.json"),serde_json::to_vec_pretty(&json!({"engine":"rust","stage":stage,"q":q,"preparedTiles":done,"totalTiles":total,"pid":std::process::id(),"updated":now(),"elapsedSeconds":start.elapsed().as_secs_f64(),"complete":false,"distributable":false}))?)?;Ok(())
}
fn borders(db:&Connection,q:i64,j:Job)->Result<BTreeMap<Point,f64>> {
    let s=j.n() as i64-1;let (x,y)=(j.x*s,j.y*s);
    let mut stmt=db.prepare("SELECT x,y,z FROM borders WHERE q=?1 AND ((x IN (?2,?3) AND y BETWEEN ?4 AND ?5) OR (y IN (?4,?5) AND x BETWEEN ?2 AND ?3)) ORDER BY x,y")?;
    Ok(stmt.query_map(params![q,x,x+s,y,y+s],|r|Ok(([r.get::<_,i64>(0)?-x,r.get::<_,i64>(1)?-y],r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
}
fn border(db:&Connection,q:i64,p:Point,z:f64)->Result<bool> {
    let old:Option<f64>=db.query_row("SELECT z FROM borders WHERE q=?1 AND x=?2 AND y=?3",params![q,p[0],p[1]],|r|r.get(0)).optional()?;
    if let Some(old)=old {ensure!(old==z,"共有辺の標高が一致しません Q{q} {p:?}: {old} / {z}");return Ok(false);}
    db.execute("INSERT INTO borders VALUES(?1,?2,?3,?4)",params![q,p[0],p[1],z])?;Ok(true)
}
fn keys(db:&Connection,q:i64)->Result<Vec<Job>> {let mut stmt=db.prepare("SELECT x,y FROM basic WHERE q=?1 ORDER BY y,x")?;Ok(stmt.query_map([q],|r|Ok(Job{q,x:r.get(0)?,y:r.get(1)?}))?.collect::<rusqlite::Result<_>>()?)}
fn ready(db:&Connection,key:&str)->Result<bool> {Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key=?1)",[key],|r|r.get(0))?)}
fn canonical(f:Face)->Face {let i=(0..3).min_by_key(|&i|f[i]).unwrap();[f[i],f[(i+1)%3],f[(i+2)%3]]}

// 親から引き継ぐ部分を先頭に並べる。親番号は、すでに保存した親の並びを参照する。
fn pack(db:&Connection,j:Job,g:&Geometry,parent:Option<(Job,&Geometry)>)->Result<Geometry> {
    ensure!(g.p.len()<=65535,"16ビットの頂点上限です");let s=j.n() as i64-1;
    ensure!(g.p.len()==g.z.len()&&g.f.iter().all(|f|f.iter().all(|&i|i<g.p.len())),"形状の参照が不正です");
    let mut area=0i128;
    for &f in &g.f {let [a,b,c]=f.map(|i|g.p[i]);let determinant=(b[0]-a[0]) as i128*(c[1]-a[1]) as i128-(b[1]-a[1]) as i128*(c[0]-a[0]) as i128;
        ensure!(determinant>0&&adaptive_tin::quality_tin::minimum_angle(&g.p,f)>=10.-1e-6,"保存前の向き・角度検証が未達です");area+=determinant;
    }
    ensure!(area==2*s as i128*s as i128,"保存前の被覆面積が一致しません");
    let by_point=g.p.iter().enumerate().map(|(i,&p)|(p,i)).collect::<BTreeMap<_,_>>();
    ensure!(by_point.len()==g.p.len(),"頂点の重複があります");
    let mut order=Vec::new();let mut refs=Vec::new();let mut used=HashSet::new();
    if let Some((pj,pg))=parent {
        for (id,&p) in pg.p.iter().enumerate() {let local=[p[0]-(j.x-pj.x*2)*s,p[1]-(j.y-pj.y*2)*s];
            if local.iter().all(|&v|v>=0&&v<=s) {let &child=by_point.get(&local).context("子に親の頂点がありません")?;ensure!(g.z[child]==pg.z[id],"親から標高が変わりました");refs.extend((id as u16).to_le_bytes());order.push(child);used.insert(child);}
        }
    }
    let inherited=order.len();for i in 0..g.p.len() {if !used.contains(&i) {order.push(i);}}
    let mut reverse=vec![0;g.p.len()];for (new,&old) in order.iter().enumerate(){reverse[old]=new;}
    let packed=Geometry{p:order.iter().map(|&i|g.p[i]).collect(),z:order.iter().map(|&i|g.z[i]).collect(),f:g.f.iter().map(|f|f.map(|i|reverse[i])).collect()};
    let mut new_vertices=Vec::new();
    for i in inherited..packed.p.len() {let p=packed.p[i];let z=packed.z[i]*4.;ensure!(p.iter().all(|&v|(0..=65535).contains(&v))&&z.is_finite()&&z==z.round()&&(-32768. ..=32767.).contains(&z),"16ビットに損失なく保存できません");for v in p {new_vertices.extend((v as u16).to_le_bytes());}new_vertices.extend((z as i16).to_le_bytes());}
    let indices=triangle_frontier::encode(&packed.f)?;
    let mut restored=triangle_frontier::decode(&indices,packed.f.len(),packed.p.len())?.into_iter().map(canonical).collect::<Vec<_>>();let mut expected=packed.f.iter().copied().map(canonical).collect::<Vec<_>>();restored.sort();expected.sort();ensure!(restored==expected,"TFC1で面が変化しました");
    let mut border_indices=Vec::new();for (axis,value) in [(0,0),(0,s),(1,0),(1,s)] {let mut ids=(0..packed.p.len()).filter(|&i|packed.p[i][axis]==value).collect::<Vec<_>>();ids.sort_by_key(|&i|packed.p[i][1-axis]);border_indices.extend((ids.len() as u32).to_le_bytes());for id in ids {border_indices.extend((id as u16).to_le_bytes());}}
    let pj=parent.map(|p|p.0).unwrap_or(Job{q:-1,x:0,y:0});
    let tx=db.unchecked_transaction()?;
    tx.execute("INSERT OR REPLACE INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![j.q,j.x,j.y,pj.q,pj.x,pj.y,packed.p.len(),packed.f.len(),inherited,refs,new_vertices,indices,border_indices])?;
    let check=restore(&tx,j)?;ensure!(check.p==packed.p&&check.z==packed.z,"頂点差分の復元が一致しません");tx.commit()?;Ok(packed)
}
fn restore(db:&Connection,j:Job)->Result<Geometry> {
    let (pq,px,py,nv,nf,inherited,refs,body,indices):(i64,i64,i64,usize,usize,usize,Vec<u8>,Vec<u8>,Vec<u8>)=db.query_row("SELECT parent_q,parent_x,parent_y,vertices,triangles,inherited,parent_refs,new_vertices,indices FROM tiles WHERE q=?1 AND x=?2 AND y=?3",params![j.q,j.x,j.y],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))?;
    ensure!(nv<=65535 && inherited<=nv && refs.len()==inherited*2 && body.len()==(nv-inherited)*6,"頂点差分の長さが不正です");
    let mut g=Geometry{p:Vec::with_capacity(nv),z:Vec::with_capacity(nv),f:Vec::new()};let s=j.n() as i64-1;
    if pq>=0 {ensure!(pq==j.q+1&&pq<=12&&px==j.x.div_euclid(2)&&py==j.y.div_euclid(2),"親タイル参照が不正です");let pg=restore(db,Job{q:pq,x:px,y:py})?;
        let mut previous=None;for r in refs.chunks_exact(2) {let id=u16::from_le_bytes(r.try_into()?) as usize;ensure!(id<pg.p.len()&&previous.is_none_or(|v|id>v),"親頂点の参照順が不正です");previous=Some(id);g.p.push([pg.p[id][0]-(j.x-px*2)*s,pg.p[id][1]-(j.y-py*2)*s]);g.z.push(pg.z[id]);}
    }else{ensure!(inherited==0,"最上位に親参照があります");}
    for p in body.chunks_exact(6) {g.p.push([u16::from_le_bytes(p[0..2].try_into()?) as i64,u16::from_le_bytes(p[2..4].try_into()?) as i64]);g.z.push(i16::from_le_bytes(p[4..6].try_into()?) as f64/4.);}
    ensure!(g.p.iter().all(|p|p.iter().all(|&v|v>=0&&v<=s))&&g.p.iter().collect::<HashSet<_>>().len()==nv,"差分復元した頂点が重複または範囲外です");g.f=triangle_frontier::decode(&indices,nf,nv)?;Ok(g)
}

pub fn run(args:&Args)->Result<()> {
    ensure!((4..=12).contains(&args.minimum_q)&&(args.minimum_q..=12).contains(&args.maximum_q)&&args.limit.is_none(),"階層生成の範囲・制限が不正です");
    ensure!(args.density_free_vertices.is_finite()&&args.density_free_vertices>=0.&&args.min_free_gib.is_finite()&&args.min_free_gib>=0.,"密度・空き容量の設定が不正です");
    let original_path=args.base_input.as_ref().context("基本入力が必要です")?;let original=read(original_path)?;
    let pending:i64=original.query_row("SELECT count(*) FROM jobs WHERE state NOT IN (1,2)",[],|r|r.get(0))?;ensure!(pending==0,"元の基本メッシュが未完了です");
    let manifest:String=original.query_row("SELECT value FROM metadata WHERE key='manifest'",[],|r|r.get(0))?;let manifest_value:Value=serde_json::from_str(&manifest)?;
    ensure!(Path::new(manifest_value["source"].as_str().context("元標高の記録がありません")?).canonicalize()?==args.source.canonicalize()?,"元標高のパスが異なります");
    let meta=fs::metadata(&args.source)?;ensure!(manifest_value["sourceSize"].as_u64()==Some(meta.len())&&manifest_value["sourceMtimeNs"].as_u64()==Some(meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64),"元標高が変更されています");
    let parent_dir=args.output.parent().unwrap_or(Path::new("."));fs::create_dir_all(parent_dir)?;
    ensure!(!args.output.exists()||args.output.canonicalize()?!=original_path.canonicalize()?,"基本入力と階層出力は分けてください");
    let mut options=OpenOptions::new();options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.share_mode(0);}
    let _lock=options.open(args.output.with_extension("lock")).context("同じ階層出力が実行中です")?;
    let final_db=Connection::open(&args.output)?;final_db.busy_timeout(Duration::from_secs(30))?;
    let count:i64=final_db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'",[],|r|r.get(0))?;
    let config=json!({"policy":"rust-hierarchical-final-parent-v1","base":original_path.canonicalize()?.to_string_lossy(),"baseBytes":fs::metadata(original_path)?.len(),"baseMtimeNs":fs::metadata(original_path)?.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as u64,"manifest":manifest_value,"minimumQ":args.minimum_q,"maximumQ":args.maximum_q,"densityFreeVertices":args.density_free_vertices,"additionalFraction":0.2,"minimumAngle":10,"triangleCodec":"TFC1","outerCompression":"none"}).to_string();
    if count>0 {let old:String=final_db.query_row("SELECT value FROM metadata WHERE key='hierarchy_config'",[],|r|r.get(0)).context("階層出力ではない既存SQLiteです")?;ensure!(old==config,"階層生成の設定が異なります");}
    final_db.execute_batch(SCHEMA)?;final_db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;CREATE TABLE IF NOT EXISTS borders(q INTEGER,x INTEGER,y INTEGER,z REAL,PRIMARY KEY(q,x,y));CREATE INDEX IF NOT EXISTS borders_y ON borders(q,y,x);")?;
    for (k,v) in [("hierarchy_config",config.as_str()),("complete","false"),("distributable","false"),("xyUnitMeters","31.25"),("heightUnitMeters","0.25"),("triangleEncoding","TFC1"),("outerCompression","none"),("boundaryConnectionComplete","false")] {final_db.execute("INSERT OR IGNORE INTO metadata VALUES(?1,?2)",[k,v])?;}
    let work=args.output.with_extension("work");fs::create_dir_all(&work)?;let start=Instant::now();
    for q in (args.minimum_q..=args.maximum_q).rev() {
        if ready(&final_db,&format!("q{q}-complete"))? {continue;}
        if stopped(args){status(args,"paused",q,0,0,start)?;return Ok(());}
        ensure!(resources::disk(parent_dir)? as f64>=args.min_free_gib*1073741824.,"空き容量が停止条件を下回りました");
        let jobs=keys(&original,q)?;ensure!(!jobs.is_empty(),"Q{q}の基本タイルがありません");let stride=Job{q,x:0,y:0}.n() as i64-1;
        let base_path=work.join(format!("q{q}-basic.sqlite3"));let pool_path=work.join(format!("q{q}-additional.sqlite3"));
        let base=Connection::open(&base_path)?;
        let tables:i64=base.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'",[],|r|r.get(0))?;
        if tables>0 {let old:String=base.query_row("SELECT value FROM metadata WHERE key='hierarchy_config'",[],|r|r.get(0)).context("階層基本用ではない既存SQLiteです")?;ensure!(old==config,"階層基本の設定が異なります");}
        base.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER,failure TEXT,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS basic(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,source_validity BLOB,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS basic_stats(q INTEGER,x INTEGER,y INTEGER,engine TEXT,max_error REAL,min_angle REAL,rounds INTEGER,PRIMARY KEY(q,x,y));")?;
        base.execute("INSERT OR IGNORE INTO metadata VALUES('hierarchy_config',?1)",[&config])?;
        base.execute("INSERT OR IGNORE INTO metadata VALUES('manifest',?1)",[&manifest])?;
        if !ready(&base,"hierarchy-basic-complete")? {
            // 親の最終頂点を先に登録し、その後で元の共有辺を加える。
            if q<args.maximum_q {for pj in keys(&original,q+1)? {let pg=restore(&final_db,pj)?;for (p,z) in pg.p.iter().zip(pg.z) {let world=[pj.x*stride*2+p[0],pj.y*stride*2+p[1]];if world[0].rem_euclid(stride)==0||world[1].rem_euclid(stride)==0 {border(&final_db,q,world,z)?;}}}}
            for &j in &jobs {let g=finished(&original,None,j)?;for (p,z) in g.p.iter().zip(g.z) {if p[0]==0||p[1]==0||p[0]==stride||p[1]==stride {border(&final_db,q,[j.x*stride+p[0],j.y*stride+p[1]],z)?;}}base.execute("INSERT OR IGNORE INTO jobs VALUES(?1,?2,?3,0,NULL)",params![q,j.x,j.y])?;}
            let mut source=Source::open(&args.source)?;let mut remaining=true;
            while remaining {
                remaining=false;
                for (index,&j) in jobs.iter().enumerate() {
                    if stopped(args){status(args,"paused",q,index,jobs.len(),start)?;return Ok(());}
                    let done:i64=base.query_row("SELECT state FROM jobs WHERE q=?1 AND x=?2 AND y=?3",params![q,j.x,j.y],|r|r.get(0))?;if done==1 {continue;}
                    status(args,"hierarchical-building-basic",q,index,jobs.len(),start)?;
                    let mut g=finished(&original,None,j)?;let mut required=borders(&final_db,q,j)?;
                    if q<args.maximum_q {let pj=Job{q:q+1,x:j.x.div_euclid(2),y:j.y.div_euclid(2)};let pg=restore(&final_db,pj)?;
                        for (p,z) in pg.p.iter().zip(pg.z) {let local=[p[0]-(j.x-pj.x*2)*stride,p[1]-(j.y-pj.y*2)*stride];if local.iter().all(|&v|v>=0&&v<=stride) {if let Some(old)=required.insert(local,z) {ensure!(old==z,"親と共有辺の標高が異なります");}}}
                    }
                    let existing=g.p.iter().enumerate().map(|(i,&p)|(p,i)).collect::<BTreeMap<_,_>>();let mut changed=false;
                    for (&p,&z) in &required {if let Some(&i)=existing.get(&p) {ensure!(g.z[i]==z,"既存頂点と親の標高が異なります");}else{g.p.push(p);g.z.push(z);changed=true;}}
                    let (error,angle,rounds)=if changed {
                        ensure!(j.memory()<resources::memory()?*7/10,"階層生成の作業メモリーが不足しています");
                        let (grid,_,_)=source.grid(j)?;
                        for (&p,&z) in &required {ensure!(grid[p[1] as usize*j.n()+p[0] as usize]==z,"親と元標高が一致しません");}
                        let refined=adaptive_tin::quality_tin::refine_seeded(&grid,j.n(),g.p,j.error(),10.,||stopped(args))?;
                        g=Geometry{z:refined.points.iter().map(|p|grid[p[1] as usize*j.n()+p[0] as usize]).collect(),p:refined.points,f:refined.faces};(refined.max_error,refined.minimum_angle,refined.rounds)
                    }else{base_stats(&original,j)?};
                    ensure!(g.p.len()<=65535&&error<=j.error()+1e-6&&angle>=10.-1e-6,"階層基本の品質条件が未達です");
                    let tx=final_db.unchecked_transaction()?;
                    for (&p,&z) in g.p.iter().zip(&g.z) {if p[0]==0||p[1]==0||p[0]==stride||p[1]==stride {if border(&tx,q,[j.x*stride+p[0],j.y*stride+p[1]],z)? {
                        // 新しい辺の点を持たない隣タイルを再評価する。
                        for dy in -1..=1 {for dx in -1..=1 {if dx==0&&dy==0 {continue;}let local=[p[0]-dx*stride,p[1]-dy*stride];if local.iter().all(|&v|v>=0&&v<=stride) {base.execute("UPDATE jobs SET state=0 WHERE q=?1 AND x=?2 AND y=?3",params![q,j.x+dx,j.y+dy])?;remaining=true;}}}
                    }}}
                    tx.commit()?;
                    let validity:Vec<u8>=original.query_row("SELECT source_validity FROM basic WHERE q=?1 AND x=?2 AND y=?3",params![q,j.x,j.y],|r|r.get(0))?;
                    let tx=base.unchecked_transaction()?;tx.execute("INSERT OR REPLACE INTO basic VALUES(?1,?2,?3,?4,?5,?6,?7)",params![q,j.x,j.y,g.p.len(),g.f.len(),body(&g)?,validity])?;
                    tx.execute("INSERT OR REPLACE INTO basic_stats VALUES(?1,?2,?3,'rust-hierarchy',?4,?5,?6)",params![q,j.x,j.y,error,angle,rounds])?;tx.execute("UPDATE jobs SET state=1 WHERE q=?1 AND x=?2 AND y=?3",params![q,j.x,j.y])?;tx.commit()?;
                }
            }
            // 確定した全共有点を各タイルが持つことを保存前に検証する。
            for &j in &jobs {let g=finished(&base,None,j)?;let map=g.p.iter().copied().zip(g.z).collect::<BTreeMap<_,_>>();for (p,z) in borders(&final_db,q,j)? {ensure!(map.get(&p)==Some(&z),"共有辺の最終検証が未達です");}}
            base.execute("INSERT INTO metadata VALUES('hierarchy-basic-complete','true')",[])?;base.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        }
        drop(base);
        status(args,"hierarchical-additional",q,0,jobs.len(),start)?;
        let mut pool_args=args.clone();pool_args.hierarchical=false;pool_args.additional=true;pool_args.base_input=Some(base_path.clone());pool_args.output=pool_path.clone();
        q_pool::run(&pool_args)?;
        let pool=read(&pool_path)?;if !ready(&pool,&format!("q{q}-complete"))? {status(args,"paused",q,0,jobs.len(),start)?;return Ok(());}
        let base=read(&base_path)?;
        for (index,&j) in jobs.iter().enumerate() {if stopped(args){status(args,"paused",q,index,jobs.len(),start)?;return Ok(());}status(args,"hierarchical-packing",q,index,jobs.len(),start)?;
            let g=finished(&base,Some(&pool),j)?;let pg=if q<args.maximum_q {let pj=Job{q:q+1,x:j.x.div_euclid(2),y:j.y.div_euclid(2)};Some((pj,restore(&final_db,pj)?))}else{None};pack(&final_db,j,&g,pg.as_ref().map(|(j,g)|(*j,g)))?;
        }
        final_db.execute("INSERT INTO metadata VALUES(?1,?2)",params![format!("q{q}-complete"),json!({"tiles":jobs.len(),"parentVerticesRetained":true,"sameLevelBordersVerified":true}).to_string()])?;
    }
    status(args,"hierarchical-ready-for-boundary-validation",args.minimum_q,0,0,start)?;Ok(())
}
fn base_stats(db:&Connection,j:Job)->Result<(f64,f64,usize)> {Ok(db.query_row("SELECT max_error,min_angle,rounds FROM basic_stats WHERE q=?1 AND x=?2 AND y=?3",params![j.q,j.x,j.y],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?)}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn delta_preserves_parent_on_negative_tile_coordinates() {
        let db=Connection::open_in_memory().unwrap();db.execute_batch(SCHEMA).unwrap();
        let parent=Job{q:5,x:-1,y:-1};let pg=Geometry{p:vec![[0,0],[128,0],[128,128],[0,128],[64,64]],z:vec![0.,1.,2.,3.,10.],f:vec![[0,1,4],[1,2,4],[2,3,4],[3,0,4]]};
        let pg=pack(&db,parent,&pg,None).unwrap();let child=Job{q:4,x:-1,y:-1};let g=Geometry{p:vec![[0,0],[64,0],[64,64],[0,64]],z:vec![10.,1.,2.,3.],f:vec![[0,1,2],[0,2,3]]};
        let got=pack(&db,child,&g,Some((parent,&pg))).unwrap();assert_eq!(got.z[0],2.);assert_eq!(restore(&db,child).unwrap().p,got.p);
        let mut missing=g;missing.z[0]=11.;assert!(pack(&db,child,&missing,Some((parent,&pg))).is_err());
    }
    #[test] fn regional_pipeline_when_fixture_is_requested() {
        let Ok(input)=std::env::var("TERRAIN_HIERARCHY_SMOKE_INPUT") else {return};
        let output=PathBuf::from(std::env::var("TERRAIN_HIERARCHY_SMOKE_OUTPUT").expect("試験出力を指定してください"));
        assert!(!output.exists(),"試験は新しい出力名で実行してください");
        let original_bytes=fs::read(&input).unwrap();let db=read(Path::new(&input)).unwrap();
        let manifest:String=db.query_row("SELECT value FROM metadata WHERE key='manifest'",[],|r|r.get(0)).unwrap();let manifest:Value=serde_json::from_str(&manifest).unwrap();
        let source=manifest["source"].as_str().unwrap();
        let mut args=Args::try_parse_from(["terrain-q-national","--hierarchical","--base-input",&input,"--output",output.to_str().unwrap(),"--source",source,"--minimum-q","6","--maximum-q","8","--density-free-vertices","4096","--max-additional-seconds","0"]).unwrap();
        args.field_cache_input=std::env::var_os("TERRAIN_HIERARCHY_SMOKE_CACHE").map(PathBuf::from);
        run(&args).unwrap();assert_eq!(fs::read(&input).unwrap(),original_bytes);
        let final_db=read(&output).unwrap();let mut tiles=0;
        for q in 6..=8 {assert!(ready(&final_db,&format!("q{q}-complete")).unwrap());
            for j in keys(&db,q).unwrap() {let g=restore(&final_db,j).unwrap();assert!(g.p.len()<=65535);
                let area:i128=g.f.iter().map(|f|{let [a,b,c]=f.map(|i|g.p[i]);(b[0]-a[0]) as i128*(c[1]-a[1]) as i128-(b[1]-a[1]) as i128*(c[0]-a[0]) as i128}).sum();assert_eq!(area,2*(j.n()-1).pow(2) as i128);
                assert!(g.f.iter().all(|&f|adaptive_tin::quality_tin::minimum_angle(&g.p,f)>=10.-1e-6));tiles+=1;
            }
        }
        assert_eq!(tiles,63);let distributable:String=final_db.query_row("SELECT value FROM metadata WHERE key='distributable'",[],|r|r.get(0)).unwrap();assert_eq!(distributable,"false");
    }
}
