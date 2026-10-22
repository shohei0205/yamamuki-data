/* ASTC 8×8の全ミップを検証し、対応GPUへ圧縮したまま渡す。 */
(function(root){
function parse(buffer,size,preview=false){
 const bytes=new Uint8Array(buffer),view=new DataView(buffer);let at=12;
 if(bytes.length<12||String.fromCharCode(...bytes.subarray(0,4))!==(preview?'LCP1':'LCA1')||view.getUint32(4,true)!==size||view.getUint32(8,true)!==Math.log2(size)+1)throw Error('地表ASTCの形式または解像度が不正です');
 const levels=[];
 for(let n=size;n>=1;n=Math.floor(n/2)){
  if(at+12>bytes.length)throw Error('地表ASTCのミップが不足しています');
  const w=view.getUint32(at,true),h=view.getUint32(at+4,true),length=view.getUint32(at+8,true);at+=12;
  if(w!==n||h!==n||!length||at+length>bytes.length||(!preview&&length!==Math.ceil(n/8)**2*16))throw Error('地表ASTCのミップ寸法または長さが不正です');
  levels.push({size:n,data:bytes.subarray(at,at+length)});at+=length;
 }
 if(at!==bytes.length)throw Error('地表ASTCの末尾に余分なデータがあります');return levels;
}
function upload(gl,extension,levels){
 const texture=gl.createTexture();gl.activeTexture(gl.TEXTURE5);gl.bindTexture(gl.TEXTURE_2D,texture);
 try{
  for(let i=0;i<levels.length;i++){const m=levels[i];gl.compressedTexImage2D(gl.TEXTURE_2D,i,extension.COMPRESSED_RGBA_ASTC_8x8_KHR,m.size,m.size,0,m.data);}
  const error=gl.getError();if(error!==gl.NO_ERROR)throw Error('ASTCのGPU読み込みに失敗しました（'+error+'）');
  return texture;
 }catch(e){gl.deleteTexture(texture);throw e;}
}
root.LandcoverAstc={parse,upload};
})(typeof window==='undefined'?globalThis:window);
