// 視野の端まで先読みする判定と、移動中にも使える応答の判定を確認する。
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('tools/terrain/viewer.html','utf8');
const functions=source.slice(source.indexOf('function viewFootprint(){'),source.indexOf('function viewRequest(){'));
const bounds=source.slice(source.indexOf('function boundsOf(points){'),source.indexOf('let loadedBounds='));
const controls={zoom:{value:'1.7'},tilt:{value:'35'},bearing:{value:'180'}};
const context={Math,baseSpan:39418,baseCameraDistance:40434.82,canvas:{clientWidth:1000,clientHeight:800},viewCenter:[-2977,727,0],document:{getElementById:id=>controls[id]}};
vm.createContext(context);vm.runInContext(bounds+functions,context);
const footprint=context.viewFootprint();
assert.equal(context.boundsContain([-19709,-15782,19709,15753],footprint.bounds),false,'初期の狭い範囲では画面の遠方を覆えていない');
const half=footprint.size*500;
assert.equal(context.boundsContain([-2977-half,727-half,-2977+half,727+half],footprint.bounds),true);
for(const tilt of ['0','35','65','90'])for(const bearing of ['0','45','180','270']){
 controls.tilt.value=tilt;controls.bearing.value=bearing;const p=context.viewFootprint();
 assert.ok(p.bounds.every(Number.isFinite));assert.ok(p.size>=2&&p.size<=4000);
 const h=p.size*500;assert.equal(context.boundsContain([-2977-h,727-h,-2977+h,727+h],p.bounds),true);
}
const request={manual:false,z:11,viewKey:'35,180,23785',coverageBounds:[-100,-100,100,100]};
const current={manual:false,z:11,viewKey:request.viewKey,bounds:[-80,-70,90,80]};
assert.equal(context.usefulPanResponse(request,current),true);
for(const changed of [{bounds:[-80,-70,110,80]},{z:12},{manual:true},{viewKey:'40,180,23785'}])assert.equal(context.usefulPanResponse(request,{...current,...changed}),false);
assert.equal(context.usefulPanResponse({...request,z:8,targetZ:11},current),true);
console.log('画面四隅の先読み、地平線、回転、移動中の応答利用と範囲外応答の除外を確認しました。');
