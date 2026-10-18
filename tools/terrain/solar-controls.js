/* JST の日時指定と、従来の手動光源を切り替える。 */
'use strict';
window.SolarControls=class {
 constructor({manualIds,location,onChange}){
  this.coefficients=SolarPosition.ephemeris(new Date());this.location=location;this.onChange=onChange;this.manualIds=manualIds;
  const row=document.createElement('div');row.style.cssText='width:100%;display:flex;gap:10px;align-items:center;flex-wrap:wrap';
  row.innerHTML='<label>太陽の指定 <select id="sunMode"><option value="datetime" selected>日時（JST）</option><option value="manual">方角・高さを手動指定</option></select></label><label>日時 <input id="sunDateTime" type="datetime-local" min="1901-01-01T00:00" max="2099-12-31T23:59" step="60"></label><button id="sunPrevious">−1 時間</button><button id="sunNext">＋1 時間</button><button id="sunNow">現在</button><span id="solarStatus" style="font-size:12px" role="status"></span>';
  document.querySelector('header').append(row);this.mode=document.getElementById('sunMode');this.input=document.getElementById('sunDateTime');
  const params=new URL(window.location.href).searchParams;
  this.input.value=params.get('sunDateTime')||SolarPosition.formatJST(new Date());if(params.get('sunMode')==='manual')this.mode.value='manual';
  this.update=()=>{try{if(this.enabled)this.coefficients=SolarPosition.ephemeris(SolarPosition.parseJST(this.input.value));this.error='';}catch(e){this.error=e.message;}for(const id of manualIds)document.getElementById(id).disabled=this.enabled;this.input.disabled=!this.enabled;for(const id of ['sunPrevious','sunNext','sunNow'])document.getElementById(id).disabled=!this.enabled;this.describe();if(!this.error){const url=new URL(window.location.href);url.searchParams.set('sunMode',this.mode.value);url.searchParams.set('sunDateTime',this.input.value);history.replaceState(null,'',url);}onChange();};
  this.mode.addEventListener('change',this.update);this.input.addEventListener('input',this.update);
  document.getElementById('sunNow').onclick=()=>{this.input.value=SolarPosition.formatJST(new Date());this.update();};
  for(const [id,hours]of [['sunPrevious',-1],['sunNext',1]])document.getElementById(id).onclick=()=>{try{this.input.value=SolarPosition.formatJST(new Date(SolarPosition.parseJST(this.input.value).getTime()+hours*3600000));this.update();}catch(e){document.getElementById('solarStatus').textContent=e.message;}};
  this.update();
 }
 get enabled(){return this.mode.value==='datetime';}
 describe(){const node=document.getElementById('solarStatus');if(this.error){node.textContent=this.error;return;}if(!this.enabled){node.textContent='手動の光源を全域で共用';return;}const point=this.location(),sun=SolarPosition.position(this.coefficients,point.latitude,point.longitude);node.textContent=`JST（UTC+9）／表示中心 ${point.latitude.toFixed(2)}°, ${point.longitude.toFixed(2)}°：方角 ${sun.azimuth.toFixed(1)}°・高さ ${sun.altitude.toFixed(1)}°${sun.altitude<0?'（地平線下）':''}。各地点の緯度・経度で個別計算。`;}
 request(){if(this.error)throw Error(this.error);return this.enabled?{datetime:this.input.value}:null;}
 uniforms(gl,locations,bounds){gl.uniform1i(locations.solarEnabled,this.enabled);gl.uniform2f(locations.solarAngles,this.coefficients.declination,this.coefficients.hourAngle);gl.uniform4f(locations.solarBounds,...bounds);this.describe();}
};
