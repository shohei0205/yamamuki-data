import {createVariableMeshWasm} from './variable-wasm-core.mjs';
const ready=fetch('/variable-mesh.wasm').then(r=>{if(!r.ok)throw Error('WASMを取得できません');return r.arrayBuffer();}).then(createVariableMeshWasm);
self.onmessage=async event=>{const{id,tiles,join}=event.data;try{const build=await ready,result=build(tiles,join);self.postMessage({id,...result},[result.vertices,result.indices]);}catch(e){self.postMessage({id,error:e.message});}};
