//! 全国追加の前に、基本メッシュからの復元と実際の挿入を計測する。
use super::*;

pub(super) fn decode(raw:&[u8],n:usize)->Result<(Vec<[usize;2]>,Vec<[usize;3]>)> {
    ensure!(raw.len()>=12 && &raw[..4]==b"YQB1","基本メッシュの形式が不正です");
    let nv=u32::from_le_bytes(raw[4..8].try_into()?) as usize;
    let nf=u32::from_le_bytes(raw[8..12].try_into()?) as usize;
    ensure!(nv>=4 && raw.len()==12+nv*12+nf*12,"基本メッシュのサイズが不正です");
    let mut points=Vec::with_capacity(nv);
    for i in 0..nv {let mut p=[0;2];for k in 0..2 {let v=f32::from_le_bytes(raw[12+i*12+k*4..16+i*12+k*4].try_into()?) as f64/SPACING;ensure!(v.is_finite()&&v>=0.&&v<n as f64&&v==v.round(),"格子外の頂点です");p[k]=v as usize;}points.push(p);}
    let mut faces=Vec::with_capacity(nf);
    for i in 0..nf {let f=std::array::from_fn(|k|u32::from_le_bytes(raw[12+nv*12+i*12+k*4..16+nv*12+i*12+k*4].try_into().unwrap()) as usize);ensure!(f.iter().all(|&v|v<nv),"頂点参照が範囲外です");faces.push(f);}
    Ok((points,faces))
}

pub fn run(args:&Args)->Result<()> {
    let base=args.base_input.as_ref().context("基本入力が必要です")?;
    q_additional_store::prepare(base,&args.output)?;
    let db=Connection::open(&args.output)?;db.busy_timeout(Duration::from_secs(30))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS benchmark(q INTEGER,x INTEGER,y INTEGER,basic_vertices INTEGER,added INTEGER,source_seconds REAL,restore_seconds REAL,insert_seconds REAL,save_seconds REAL,payload BLOB,PRIMARY KEY(q,x,y));")?;
    let input=read(base)?;let mut source=Source::open(&args.source)?;
    let started=Instant::now();let mut reports=Vec::new();
    // 最小・中位・最大の頂点数を持つタイルを各レベルから選ぶ。巨大格子は実測の走査速度から別途推計する。
    for q in 4..=9 {
        let count:usize=input.query_row("SELECT count(*) FROM basic WHERE q=?1",[q],|r|r.get(0))?;
        for offset in [0,count/2,count.saturating_sub(1)] {
            let row=input.query_row("SELECT x,y,vertices,mesh FROM basic WHERE q=?1 ORDER BY vertices,x,y LIMIT 1 OFFSET ?2",params![q,offset],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,usize>(2)?,r.get::<_,Vec<u8>>(3)?)))?;
            let (x,y,nv,blob)=row;let job=Job{q,x,y};let t=Instant::now();
            let (grid,_bits,_)=source.grid(job)?;let source_seconds=t.elapsed().as_secs_f64();
            let mut raw=Vec::new();ZlibDecoder::new(&blob[..]).read_to_end(&mut raw)?;let (points,faces)=decode(&raw,job.n())?;
            let limits=vec![job.error();grid.len()];let t=Instant::now();let mut builder=adaptive_tin::Builder::new(&grid,job.n(),job.n())?.with_error_limits(&limits)?.with_seed_mesh(&points,&faces)?.with_locked_boundary()?.start_detail((job.n()-1) as f64/2.,4096.,10.,job.error())?.with_detail_priority_floor(0.1)?;
            let restore_seconds=t.elapsed().as_secs_f64();let t=Instant::now();let mut added=0;
            for _ in 0..nv.div_ceil(5).min(256) {if !builder.detail_step()? {break;}added+=1;}
            let insert_seconds=t.elapsed().as_secs_f64();let t=Instant::now();let mesh=builder.detail_snapshot()?;
            // 試験結果は全国プールの成果とは区別し、additionalテーブルへ混ぜない。
            let payload=zip(&serde_json::to_vec(&mesh)?)?;
            db.execute("INSERT OR REPLACE INTO benchmark VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,?9)",params![q,x,y,nv,added,source_seconds,restore_seconds,insert_seconds,payload])?;
            let save_seconds=t.elapsed().as_secs_f64();db.execute("UPDATE benchmark SET save_seconds=?1 WHERE q=?2 AND x=?3 AND y=?4",params![save_seconds,q,x,y])?;
            let report=json!({"q":q,"x":x,"y":y,"vertices":nv,"added":added,"sourceSeconds":source_seconds,"restoreSeconds":restore_seconds,"insertSeconds":insert_seconds,"saveSeconds":save_seconds});
            println!("追加計測 {report}");reports.push(report);
            fs::write(args.output.with_extension("benchmark.json"),serde_json::to_vec_pretty(&json!({"stage":"benchmarking","reports":reports,"elapsedSeconds":started.elapsed().as_secs_f64(),"nationalRunStarted":false}))?)?;
        }
    }
    let mut estimated=0.;let mut summary=Vec::new();
    for q in 4..=12 {let (tiles,vertices):(u64,u64)=input.query_row("SELECT count(*),sum(vertices) FROM basic WHERE q=?1",[q],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let measured=q.min(9);let selected=reports.iter().filter(|r|r["q"].as_i64()==Some(measured)).collect::<Vec<_>>();
        let source_per_pixel=selected.iter().map(|r|r["sourceSeconds"].as_f64().unwrap()).sum::<f64>()/selected.len() as f64/(Job{q:measured,x:0,y:0}.n().pow(2) as f64);
        let startup=selected.iter().map(|r|r["restoreSeconds"].as_f64().unwrap()+r["saveSeconds"].as_f64().unwrap()).sum::<f64>()/selected.len() as f64;
        let per_vertex=selected.iter().filter(|r|r["added"].as_u64().unwrap()>0).map(|r|r["insertSeconds"].as_f64().unwrap()/r["added"].as_u64().unwrap() as f64).fold(0f64,f64::max);
        let seconds=tiles as f64*(source_per_pixel*Job{q,x:0,y:0}.n().pow(2) as f64+startup)+(vertices/5) as f64*per_vertex;
        estimated+=seconds;summary.push(json!({"q":q,"tiles":tiles,"budget":vertices/5,"estimatedSerialSeconds":seconds}));
    }
    let result=json!({"stage":"benchmark-complete","reports":reports,"levels":summary,"estimatedSerialSeconds":estimated,"elapsedSeconds":started.elapsed().as_secs_f64(),"nationalRunStarted":false,"includesGlobalQueueAndCrossTileDensity":false,"optimizationRequired":estimated>=7200.});
    fs::write(args.output.with_extension("benchmark.json"),serde_json::to_vec_pretty(&result)?)?;
    db.execute("INSERT OR REPLACE INTO metadata VALUES('benchmark',?1)",[result.to_string()])?;
    println!("追加処理の単独実行推計 {:.1}時間（全国キュー・隣接密度更新は別途加算）",estimated/3600.);Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn malformed_mesh_is_rejected(){assert!(decode(b"bad",65).is_err());let mut raw=b"YQB1".to_vec();raw.extend(4u32.to_le_bytes());raw.extend(2u32.to_le_bytes());assert!(decode(&raw,65).is_err());}
}
