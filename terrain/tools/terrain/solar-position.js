/* NOAA の太陽位置計算。日時は UTC の瞬間、表示入力は固定の JST。 */
'use strict';
const SolarPosition=(()=>{
 const rad=Math.PI/180;
 function ephemeris(date){
  const t=(date.getTime()/86400000+2440587.5-2451545)/36525;
  const l=((280.46646+t*(36000.76983+t*.0003032))%360+360)%360;
  const m=(357.52911+t*(35999.05029-.0001537*t))*rad,e=.016708634-t*(.000042037+.0000001267*t);
  const c=Math.sin(m)*(1.914602-t*(.004817+.000014*t))+Math.sin(2*m)*(.019993-.000101*t)+Math.sin(3*m)*.000289;
  const omega=(125.04-1934.136*t)*rad,lambda=(l+c-.00569-.00478*Math.sin(omega))*rad;
  const ob=(23+(26+(21.448-t*(46.815+t*(.00059-t*.001813)))/60)/60+.00256*Math.cos(omega))*rad;
  const declination=Math.asin(Math.sin(ob)*Math.sin(lambda)),y=Math.tan(ob/2)**2,L=l*rad;
  const equation=4/rad*(y*Math.sin(2*L)-2*e*Math.sin(m)+4*e*y*Math.sin(m)*Math.cos(2*L)-.5*y*y*Math.sin(4*L)-1.25*e*e*Math.sin(2*m));
  const minutes=date.getUTCHours()*60+date.getUTCMinutes()+date.getUTCSeconds()/60;
  return {declination,hourAngle:((minutes+equation)/4-180)*rad,equationMinutes:equation};
 }
 function position(coeff,latitude,longitude){
  const lat=latitude*rad,h=coeff.hourAngle+longitude*rad,d=coeff.declination;
  const east=-Math.cos(d)*Math.sin(h),north=Math.sin(d)*Math.cos(lat)-Math.cos(d)*Math.cos(h)*Math.sin(lat),up=Math.sin(d)*Math.sin(lat)+Math.cos(d)*Math.cos(h)*Math.cos(lat);
  return {azimuth:(Math.atan2(east,north)/rad+360)%360,altitude:Math.asin(Math.max(-1,Math.min(1,up)))/rad,vector:[east,north,up]};
 }
 function parseJST(value){if(!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))throw Error('日時を日本標準時で指定してください');const date=new Date(value+':00+09:00');if(!Number.isFinite(date.getTime())||formatJST(date)!==value||Number(value.slice(0,4))<1901||Number(value.slice(0,4))>2099)throw Error('日時は 1901〜2099 年の有効な日時で指定してください');return date;}
 function formatJST(date){return new Date(date.getTime()+9*3600000).toISOString().slice(0,16);}
 function mercator(x,y){return {longitude:x*360-180,latitude:Math.atan(Math.sinh(Math.PI*(1-2*y)))/rad};}
 const glsl=`uniform bool solarEnabled;uniform vec2 solarAngles;uniform vec4 solarBounds;
 vec3 solarDirection(vec2 uv){vec2 tile=solarBounds.xy+uv*solarBounds.zw;float lon=tile.x*6.28318530718-3.14159265359;float q=3.14159265359*(1.0-2.0*tile.y);float lat=atan((exp(q)-exp(-q))*0.5);float h=solarAngles.y+lon,d=solarAngles.x;return vec3(-cos(d)*sin(h),sin(d)*cos(lat)-cos(d)*cos(h)*sin(lat),sin(d)*sin(lat)+cos(d)*cos(h)*cos(lat));}
 float solarDay(vec3 sun){return smoothstep(-0.104528,0.034899,sun.z);}`;
 return {ephemeris,position,parseJST,formatJST,mercator,glsl};
})();
if(typeof module!=='undefined')module.exports=SolarPosition;else window.SolarPosition=SolarPosition;
