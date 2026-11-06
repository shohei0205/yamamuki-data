const fs=require('fs'),zlib=require('zlib');
(async()=>{
 const source=fs.readFileSync('tools/terrain/landcover_bezier_decode.js','utf8');
 const {decodeBezierCoverage}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
 const rows=[],root=process.argv[2]||'build/landcover-bezier-fit-fuji-v5';
 for(const filter of fs.readdirSync(root).filter(f=>fs.existsSync(root+'/'+f+'/manifest.json'))){
  const dir=root+'/'+filter,manifest=JSON.parse(fs.readFileSync(dir+'/manifest.json'));
  for(const row of manifest.levels){
   const compressed=fs.readFileSync(dir+'/'+row.curveInfo.controlFile);
   if(row.curveInfo.controlSha256&&require('crypto').createHash('sha256').update(compressed).digest('hex')!==row.curveInfo.controlSha256)throw Error('Control hash mismatch');
   const raw=zlib.gunzipSync(compressed);
   const data=decodeBezierCoverage(raw.buffer.slice(raw.byteOffset,raw.byteOffset+raw.byteLength),row.curveInfo);
   const reference=JSON.parse(zlib.gunzipSync(fs.readFileSync(dir+'/'+row.file)));
   let count=0,max=0;
   function compare(a,b){
    if(Array.isArray(a)){if(!Array.isArray(b)||a.length!==b.length)throw Error('Shape mismatch '+filter+' L'+row.level);for(let i=0;i<a.length;i++)compare(a[i],b[i]);}
    else{max=Math.max(max,Math.abs(a-b));count++;}
   }
   for(let c=0;c<5;c++)compare(data.features[c].geometry.coordinates,reference.features[c].geometry.coordinates);
   if(max>1e-7)throw Error('Coordinate mismatch '+max);
   rows.push({filter,level:row.level,coordinates:count/2,maximumDifferenceM:max,controlBytes:row.curveInfo.controlBytes});
  }
 }console.log(JSON.stringify(rows,null,2));
})().catch(e=>{console.error(e);process.exit(1)});
