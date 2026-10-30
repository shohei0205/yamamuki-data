export const MORPH_MS=450;
export function morphWeight(start,now){if(!Number.isFinite(start))return 1;const t=Math.max(0,Math.min(1,(now-start)/MORPH_MS));return t*t*(3-2*t);}
export function coverageKey(list){return list.map(v=>[v.offset,...(v.drawBounds||v.tile.bounds)].join('/')).sort().join(';');}
