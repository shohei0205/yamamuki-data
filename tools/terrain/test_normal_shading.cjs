// 法線の平均化が平面を保ち、扇形の陰影のばらつきを減らすことを確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
async function main(){
  const b=await chromium.launch({headless:true,channel:'msedge'});
  try{
    const p=await b.newPage();
    await p.goto('file:///C:/Work/GitHub/yamamuki-org/build/terrain-tin.html');
    await p.waitForFunction(()=>document.querySelector('#stats').textContent.includes('三角形'));
    const result=await p.evaluate(()=>{
      const original=data.mesh,polygon=data.polygon;
      const mesh=[[0,0,20],[-100,-100,0],[100,-100,0],[100,100,0],[-100,100,0]];
      const faces=[0,1,2,0,2,3,0,3,4,0,4,1];
      function energy(normals){let sum=0;for(let i=0;i<faces.length;i+=3)for(let j=0;j<3;j++)for(let axis=0;axis<3;axis++){const a=faces[i+j],b=faces[i+(j+1)%3];sum+=(normals[a*3+axis]-normals[b*3+axis])**2;}return sum;}
      try{
        data.mesh=mesh.map(v=>[...v]);data.polygon=false;const before=energy(terrainNormals(faces));
        data.mesh=mesh.map(v=>[...v]);data.polygon=true;const normals=terrainNormals(faces),after=energy(normals);
        const unchanged=JSON.stringify(data.mesh)===JSON.stringify(mesh);
        const cached=terrainNormals(faces)===normals;
        let unit=true;for(let i=0;i<normals.length;i+=3)unit&&=Math.abs(Math.hypot(...normals.slice(i,i+3))-1)<1e-6;
        data.mesh=mesh.map(([x,y])=>[x,y,0]);const flat=Array.from(terrainNormals(faces));
        return {before,after,unchanged,cached,unit,flat};
      }finally{data.mesh=original;data.polygon=polygon;}
    });
    assert.ok(result.after<result.before*.5);assert.ok(result.unchanged&&result.cached&&result.unit);
    for(let i=0;i<result.flat.length;i++)assert.equal(result.flat[i],i%3===2?1:0);
    console.log(JSON.stringify({before:result.before,after:result.after,geometryUnchanged:result.unchanged}));
  }finally{await b.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});
