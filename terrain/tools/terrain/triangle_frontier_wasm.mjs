// ブラウザーとNode.jsで共通のWASM呼出し。コピーと出力配列の確保も行う。
export async function createTfcDecoder(binary){
 const {instance}=await WebAssembly.instantiate(binary,{}),e=instance.exports;
 const base=Number(e.__heap_base.value);
 return function decode(src,count,vertexCount=65536){
  if(!(src instanceof Uint8Array)||!Number.isInteger(count)||count<0||count%3||count>6000000||!Number.isInteger(vertexCount)||vertexCount<1||vertexCount>65536)throw Error('TFC1の引数が不正です');
  const target=(base+src.length+7)&~7,total=target+count*2;
  if(total>67108864||src.length>33554432)throw Error('TFC1の入力が大きすぎます');
  if(total>e.memory.buffer.byteLength)e.memory.grow(Math.ceil((total-e.memory.buffer.byteLength)/65536));
  new Uint8Array(e.memory.buffer,base,src.length).set(src);
  const code=e.tfc_decode(base,src.length,target,count,vertexCount);
  if(code!==0)throw Error(`TFC1の入力が不正です (${code})`);
  return new Uint16Array(e.memory.buffer,target,count).slice();
 };
}
