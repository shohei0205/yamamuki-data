/* 画面の右・下方向を、地形の東・南方向へ逆変換する。 */
'use strict';
const TerrainNavigation={
 offset(dx,dy,scale,height,bearing,tilt){
  const unit=2*scale/height,c=Math.cos(bearing),s=Math.sin(bearing),vertical=dy/Math.max(.01,Math.cos(tilt));
  return [(c*dx+s*vertical)*unit,(-s*dx+c*vertical)*unit];
 },
 pan(center,dx,dy,scale,height,bearing,tilt){const d=this.offset(dx,dy,scale,height,bearing,tilt);return [center[0]-d[0],center[1]-d[1]];},
 zoom(center,dx,dy,before,after,height,bearing,tilt){const a=this.offset(dx,dy,before,height,bearing,tilt),b=this.offset(dx,dy,after,height,bearing,tilt);return [center[0]+a[0]-b[0],center[1]+a[1]-b[1]];}
};
if(typeof module!=='undefined')module.exports=TerrainNavigation;
