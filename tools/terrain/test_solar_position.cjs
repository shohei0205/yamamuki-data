const assert=require('node:assert/strict'),s=require('./solar-position.js');
// NREL/TP-560-34302 の参照例。屈折を含む公表値なので高度は 0.1 度の範囲で照合する。
const ref=s.position(s.ephemeris(new Date('2003-10-17T19:30:30Z')),39.742476,-105.1786);
assert.ok(Math.abs(ref.azimuth-194.34024)<.1);assert.ok(Math.abs(ref.altitude-(90-50.11162))<.1);
assert.equal(s.parseJST('2026-03-20T06:00').toISOString(),'2026-03-19T21:00:00.000Z');
assert.equal(s.formatJST(s.parseJST('2024-02-29T12:00')),'2024-02-29T12:00');
const c=s.ephemeris(s.parseJST('2026-03-20T06:00'));assert.ok(s.position(c,35,130).altitude<0);assert.ok(s.position(c,35,145).altitude>0);
const night=s.position(s.ephemeris(s.parseJST('2026-06-21T00:00')),35,139);assert.ok(night.altitude<0);
const summer=s.position(s.ephemeris(s.parseJST('2026-06-21T12:00')),35,135),winter=s.position(s.ephemeris(s.parseJST('2026-12-21T12:00')),35,135);assert.ok(summer.altitude>winter.altitude+40);
for(const bad of ['2026-02-30T12:00','2026-02-29T12:00','2026-01-01T25:00','invalid','1800-01-01T12:00'])assert.throws(()=>s.parseJST(bad));
console.log('太陽位置参照値・JST変換・東西の夜明け・季節・夜間・入力検査: OK');
