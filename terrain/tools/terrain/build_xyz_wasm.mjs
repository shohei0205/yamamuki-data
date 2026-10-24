import fs from 'node:fs';import path from 'node:path';import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),root=path.resolve(import.meta.dirname,'../..');
const wabt=await require(path.join(root,'build/terrain-codec-tools/web/wabt/index.js'))();
const module=wabt.parseWat('xyz_decoder.wat',fs.readFileSync(path.join(import.meta.dirname,'xyz_decoder.wat'),'utf8'),{bulk_memory:true});module.resolveNames();module.validate();
const {buffer}=module.toBinary({write_debug_names:false});fs.writeFileSync(path.join(root,'build/xyz_decoder.wasm'),buffer);console.log(buffer.length+' bytes');module.destroy();
