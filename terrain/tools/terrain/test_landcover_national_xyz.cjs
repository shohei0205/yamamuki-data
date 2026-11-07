const assert = require('node:assert/strict');
const fs = require('node:fs');
const zlib = require('node:zlib');
const {DatabaseSync} = require('node:sqlite');

function merged(spans) {
    const output = [];
    for (const [start, end] of spans.sort((a,b) => a[0]-b[0] || a[1]-b[1])) {
        const last = output.at(-1);
        if (last && start <= last[1]) last[1] = Math.max(last[1], end);
        else output.push([start, end]);
    }
    return output;
}

function differenceLength(a,b) {
    let length = [...a,...b].reduce((total,[start,end]) => total+end-start,0);
    let i=0,j=0;
    while(i<a.length&&j<b.length) {
        length-=2*Math.max(0,Math.min(a[i][1],b[j][1])-Math.max(a[i][0],b[j][0]));
        if(a[i][1]<b[j][1])i++; else j++;
    }
    return length;
}

async function main() {
    const path = process.argv[2];
    assert.ok(path, 'Provide the national SQLite path');
    const script = fs.readFileSync(__dirname+'/landcover_bezier_decode.js');
    const {decodeBezierCoverage} = await import('data:text/javascript;base64,'+script.toString('base64'));
    const db = new DatabaseSync(path, {readOnly:true});
    try {
        assert.equal(db.prepare('PRAGMA integrity_check').get().integrity_check, 'ok');
        const meta = JSON.parse(db.prepare("SELECT value FROM metadata WHERE key='manifest'").get().value);
        const scale = 1048576;
        const metres = (index,z) => index*meta.worldWidthM/2**z;
        const axis = (index,z) => Math.round(metres(index,z)*scale);
        const known = new Set(db.prepare('SELECT x,y FROM tiles WHERE z=8').all().map(({x,y}) => x+','+y));
        assert.equal(known.size,meta.completeRegions);
        for (const {x,y} of meta.sourceGaps) assert.ok(!known.has(x+','+y));
        for (const level of meta.levels) {
            const {z} = level;
            const seen = new Map();
            let tiles=0, seams=0, payloads=0, maxDifference=0;
            for (const {x,y,kind,coverage_mask,payload} of db.prepare(
                'SELECT x,y,kind,coverage_mask,payload FROM tiles WHERE z=? ORDER BY y,x'
            ).iterate(z)) {
                tiles++;
                const x0=axis(x,z), y0=axis(y,z), x1=axis(x+1,z), y1=axis(y+1,z);
                if(z>=8) {
                    const factor=2**(z-8);
                    assert.equal(coverage_mask,1);
                    assert.ok(known.has(Math.floor(x/factor)+','+Math.floor(y/factor)));
                } else {
                    let expected=0;
                    for(let iy=0;iy<4;iy++)for(let ix=0;ix<4;ix++)
                        if(known.has((x*4+ix)+','+(y*4+iy))) expected|=1<<(iy*4+ix);
                    assert.equal(coverage_mask,expected);
                    assert.ok(coverage_mask);
                }
                const tileX=x,tileY=y;
                const edges=Array.from({length:4},()=>[[],[],[]]);
                const add=(side,cls,a,b)=>{ if(a!==b)edges[side][cls].push([Math.min(a,b),Math.max(a,b)]); };
                if(kind===3) {
                    assert.equal(payload,null);
                    if(z===6)assert.equal(coverage_mask,65535);
                    add(0,2,y0,y1);add(1,2,x0,x1);add(2,2,y0,y1);add(3,2,x0,x1);
                } else if(kind===0) assert.equal(payload,null);
                else {
                    assert.equal(kind,-1);
                    payloads++;
                    const raw=zlib.gunzipSync(payload);
                    const {features}=decodeBezierCoverage(raw.buffer.slice(raw.byteOffset,raw.byteOffset+raw.byteLength),{});
                    for(let cls=1;cls<=3;cls++)for(const polygon of features[cls].geometry.coordinates)for(const ring of polygon) {
                        const points=ring.map(([x,y])=>[Math.round((x+metres(tileX,z))*scale),Math.round((y+metres(tileY,z))*scale)]);
                        for(const [gx,gy] of points) assert.ok(gx>=x0&&gx<=x1&&gy>=y0&&gy<=y1,'Coordinate out of tile z'+z+'/'+x+'/'+y+' '+[gx,gy]+' bounds '+[x0,y0,x1,y1]);
                        for(let i=1;i<points.length;i++){
                            const [ax,ay]=points[i-1],[bx,by]=points[i];
                            if(ax===bx&&ax===x0)add(0,cls-1,ay,by);
                            if(ay===by&&ay===y0)add(1,cls-1,ax,bx);
                            if(ax===bx&&ax===x1)add(2,cls-1,ay,by);
                            if(ay===by&&ay===y1)add(3,cls-1,ax,bx);
                        }
                    }
                }
                const canonical=edges.map(edge=>edge.map(merged));
                for(let side=0;side<2;side++){
                    const previous=seen.get(side===0?(x-1)+','+y:x+','+(y-1));
                    if(!previous)continue;
                    let ranges=[];
                    if(z===6){
                        for(let i=0;i<4;i++){
                            const start=(side===0?y:x)*4+i;
                            const a=side===0?(x*4-1)+','+start:start+','+(y*4-1);
                            const b=side===0?x*4+','+start:start+','+y*4;
                            if(known.has(a)&&known.has(b))ranges.push([axis(start,8),axis(start+1,8)]);
                        }
                        if(!ranges.length)continue;
                    } else ranges=[[side===0?y0:x0,side===0?y1:x1]];
                    const clip=spans=>ranges.flatMap(([lo,hi])=>spans.map(([a,b])=>[Math.max(a,lo),Math.min(b,hi)]).filter(([a,b])=>a<b));
                    for(let cls=0;cls<3;cls++){
                        const delta=differenceLength(clip(canonical[side][cls]),clip(previous[side][cls]))/scale;
                        assert.ok(delta<=0.01,'Decoded seam z'+z+'/'+x+'/'+y+' side '+side+' class '+(cls+1)+': '+delta+' m');
                        maxDifference=Math.max(maxDifference,delta);
                    }
                    seams++;
                }
                seen.set(x+','+y,canonical.slice(2));
            }
            assert.equal(tiles,level.tiles);
            assert.equal(seams,level.checkedSeams);
            assert.equal(maxDifference,level.maxSeamDifferenceM);
            console.log('z'+z+': '+tiles+' tiles, '+payloads+' payloads decoded, '+seams+' adjacent boundaries verified, max '+maxDifference+' m');
        }
    } finally {
        db.close();
    }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
