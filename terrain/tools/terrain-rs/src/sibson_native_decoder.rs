//! NQC1/NDC1の共通復元器。Windows DLLとWASMを同じソースから作る。
use std::slice;
const MAX: usize = 10_000_000;
type R<T> = Result<T, ()>;
struct Reader<'a> { bytes: &'a [u8], pos: usize }
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(())?;
        let out = self.bytes.get(self.pos..end).ok_or(())?; self.pos = end; Ok(out)
    }
    fn byte(&mut self) -> R<u8> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> R<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?)) }
    fn var(&mut self) -> R<u64> {
        let mut value = 0;
        for i in 0..10 { let b = self.byte()?; if i == 9 && b > 1 { return Err(()); }
            value |= ((b & 127) as u64) << (i * 7); if b < 128 { return Ok(value); }
        } Err(())
    }
    fn end(&self) -> bool { self.pos == self.bytes.len() }
}
struct Bits<'a> { bytes: &'a [u8], bit: usize, limit: usize }
impl Bits<'_> {
    fn read(&mut self, mut width: usize) -> R<u64> {
        if self.bit.checked_add(width).ok_or(())? > self.limit { return Err(()); }
        let mut value = 0; let mut shift = 0;
        while width > 0 { let offset = self.bit & 7; let n = width.min(8 - offset);
            value |= (((self.bytes[self.bit >> 3] >> offset) as u64) & ((1 << n) - 1)) << shift;
            self.bit += n; shift += n; width -= n;
        } Ok(value)
    }
    fn unary(&mut self) -> R<u64> {
        let mut zeros = 0;
        loop { if self.bit >= self.limit { return Err(()); }
            let available = (8 - (self.bit & 7)).min(self.limit - self.bit);
            let value = self.bytes[self.bit >> 3] >> (self.bit & 7);
            let n = (value.trailing_zeros() as usize).min(available);
            self.bit += n; zeros += n as u64;
            if n < available { self.bit += 1; return Ok(zeros); }
        }
    }
}
fn signed(value: u64) -> i64 { ((value >> 1) as i64) ^ -((value & 1) as i64) }
pub(crate) fn stream(bytes: &[u8]) -> R<Vec<i64>> {
    let mut r = Reader::new(bytes); let count = usize::try_from(r.var()?).map_err(|_| ())?;
    if count > MAX { return Err(()); } let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let size = usize::try_from(r.var()?).map_err(|_| ())?; let mut b = Reader::new(r.take(size)?);
        let n = (count - out.len()).min(128);
        match b.byte()? {
            0 => {
                let base = signed(b.var()?); let width = b.byte()? as usize; if width > 63 { return Err(()); }
                let limit = n.checked_mul(width).ok_or(())?; let data = b.take((limit + 7) / 8)?;
                if !b.end() { return Err(()); } let mut bits = Bits { bytes: data, bit: 0, limit };
                for _ in 0..n { out.push(base.checked_add(bits.read(width)? as i64).ok_or(())?); }
            }
            1 => {
                let end = out.len() + n;
                while out.len() < end { let v = b.var()?;
                    if v != 0 { out.push(signed(v - 1)); } else {
                        let run = usize::try_from(b.var()?).map_err(|_| ())?;
                        if run == 0 || run > end - out.len() { return Err(()); } out.resize(out.len() + run, 0);
                    }
                } if !b.end() { return Err(()); }
            }
            2 => {
                let k = b.byte()? as usize; let limit = usize::try_from(b.var()?).map_err(|_| ())?;
                if k > 32 { return Err(()); } let data = b.take(limit.checked_add(7).ok_or(())? / 8)?;
                if !b.end() { return Err(()); } let mut bits = Bits { bytes: data, bit: 0, limit };
                for _ in 0..n { let q = bits.unary()?; if q > (u64::MAX >> k) { return Err(()); }
                    out.push(signed((q << k) | bits.read(k)?));
                } if bits.bit != limit { return Err(()); }
            }
            _ => return Err(()),
        }
    } if !r.end() { return Err(()); } Ok(out)
}
fn decode(bytes: &[u8], parents: &[f32], offsets: &[u32], out: &mut [f32]) -> R<usize> {
    let mut r = Reader::new(bytes); let magic = r.take(4)?; let n = r.u32()? as usize;
    let pc = r.u32()? as usize; let fresh = r.u32()? as usize;
    let xy = f32::from_bits(r.u32()?) as f64; let z = f32::from_bits(r.u32()?) as f64;
    if n > MAX || pc > 256 || fresh > n || out.len() / 3 < n || !xy.is_finite() || !z.is_finite() || xy <= 0. || z <= 0. { return Err(()); }
    let mut columns = Vec::new();
    while !r.end() { if columns.len() == 7 { return Err(()); } let len = r.u32()? as usize; columns.push(stream(r.take(len)?)?); }
    if magic == b"NQC1" {
        if pc != 0 || fresh != n || columns.len() != 3 || columns.iter().any(|v| v.len() != n) { return Err(()); }
        let mut previous = [0_i64; 3];
        for i in 0..n { for axis in 0..3 { previous[axis] = previous[axis].checked_add(columns[axis][i]).ok_or(())?;
            out[i * 3 + axis] = (previous[axis] as f64 * if axis == 2 { z } else { xy }) as f32;
        } }
    } else if magic == b"NDC1" {
        if columns.len() != 7 || columns[0].len() != n || columns[1..4].iter().any(|v| v.len() != n - fresh) || columns[4..7].iter().any(|v| v.len() != fresh) || offsets.len() != pc + 1 || offsets[0] != 0 || offsets.windows(2).any(|v| v[0] > v[1]) || offsets[pc] as usize > parents.len() / 3 { return Err(()); }
        let mut indices = vec![0_i64; pc]; let mut previous = [0_i64; 3]; let mut ni = 0; let mut nf = 0;
        for i in 0..n {
            match columns[0][i] {
                1 => { if nf >= fresh { return Err(()); } for axis in 0..3 { previous[axis] = previous[axis].checked_add(columns[axis + 4][nf]).ok_or(())?;
                    out[i * 3 + axis] = (previous[axis] as f64 * if axis == 2 { z } else { xy }) as f32;
                } nf += 1; }
                0 => {
                    if ni >= n - fresh { return Err(()); } let slot = usize::try_from(columns[1][ni]).map_err(|_| ())?;
                    if slot >= pc { return Err(()); } indices[slot] = indices[slot].checked_add(columns[2][ni]).ok_or(())?;
                    let index = usize::try_from(indices[slot]).map_err(|_| ())?;
                    if index >= (offsets[slot + 1] - offsets[slot]) as usize || index >= 1 << 24 { return Err(()); }
                    let source = (offsets[slot] as usize + index) * 3;
                    out[i * 3] = parents[source]; out[i * 3 + 1] = parents[source + 1];
                    out[i * 3 + 2] = (parents[source + 2] as f64 + columns[3][ni] as f64 * z) as f32; ni += 1;
                }
                _ => return Err(()),
            }
        } if ni != n - fresh || nf != fresh { return Err(()); }
    } else { return Err(()); }
    if out[..n * 3].iter().any(|v| !v.is_finite()) { return Err(()); } Ok(n)
}
#[unsafe(no_mangle)]
pub extern "C" fn codec_alloc(size: usize) -> *mut u8 { Box::into_raw(vec![0_u8; size].into_boxed_slice()) as *mut u8 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn codec_free(ptr: *mut u8, size: usize) {
    if !ptr.is_null() { unsafe { drop(Box::from_raw(slice::from_raw_parts_mut(ptr, size))); } }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn codec_decode(input: *const u8, len: usize, parent: *const f32, parent_floats: usize, offsets: *const u32, parent_count: usize, output: *mut f32, capacity: usize) -> i32 {
    if input.is_null() || output.is_null() || parent_count > 256 || capacity > MAX || len > 256 * 1024 * 1024 { return -1; }
    let data = unsafe { slice::from_raw_parts(input, len) };
    let p = if parent_floats == 0 { &[] } else { if parent.is_null() { return -1; } unsafe { slice::from_raw_parts(parent, parent_floats) } };
    let o = if parent_count == 0 { &[0_u32][..] } else { if offsets.is_null() { return -1; } unsafe { slice::from_raw_parts(offsets, parent_count + 1) } };
    let target = unsafe { slice::from_raw_parts_mut(output, capacity * 3) };
    decode(data, p, o, target).map(|n| n as i32).unwrap_or(-1)
}


/// 全国仕様の正規整数を復元する。float32の全球座標を経由しない。
pub(crate) fn decode_global(bytes:&[u8],origin:[i32;2],parents:&[Vec<[i32;3]>])->R<Vec<[i32;3]>> {
    if bytes.len()>16777216 || parents.len()>4 {return Err(());}
    let mut r=Reader::new(bytes);let magic=r.take(4)?;let n=r.u32()? as usize;let pc=r.u32()? as usize;let nf=r.u32()? as usize;
    if r.u32()?!=0.0625f32.to_bits() || r.u32()?!=0.125f32.to_bits() || n==0 || n>262144 || nf>n || pc!=parents.len(){return Err(());}
    let mut columns=Vec::new();
    while !r.end(){if columns.len()==7{return Err(());}let len=r.u32()? as usize;let b=r.take(len)?;if Reader::new(b).var()?>n as u64{return Err(());}columns.push(stream(b)?);}
    let mut out=vec![[0;3];n];
    if magic==b"NQC1" {
        if pc!=0 || nf!=n || columns.len()!=3 || columns.iter().any(|c|c.len()!=n){return Err(());}
        let mut sum=[0i64;3];for i in 0..n {for k in 0..3 {sum[k]=sum[k].checked_add(columns[k][i]).ok_or(())?;let v=sum[k].checked_add(if k<2 {origin[k] as i64}else{0}).ok_or(())?;out[i][k]=i32::try_from(v).map_err(|_|())?;}}
    }else if magic==b"NDC1" {
        if columns.len()!=7 || columns[0].len()!=n || columns[1..4].iter().any(|c|c.len()!=n-nf) || columns[4..].iter().any(|c|c.len()!=nf){return Err(());}
        let mut last=vec![0i64;pc];let mut sum=[0i64;3];let mut fresh=0;let mut inherited=0;
        for i in 0..n {match columns[0][i] {
            1=>{if fresh>=nf{return Err(());}for k in 0..3 {sum[k]=sum[k].checked_add(columns[k+4][fresh]).ok_or(())?;out[i][k]=i32::try_from(sum[k].checked_add(if k<2 {origin[k] as i64}else{0}).ok_or(())?).map_err(|_|())?;}fresh+=1;},
            0=>{if inherited>=n-nf{return Err(());}let slot=usize::try_from(columns[1][inherited]).map_err(|_|())?;if slot>=pc{return Err(());}last[slot]=last[slot].checked_add(columns[2][inherited]).ok_or(())?;let index=usize::try_from(last[slot]).map_err(|_|())?;if index>=parents[slot].len() || index>=1<<24{return Err(());}out[i]=parents[slot][index];out[i][2]=i32::try_from((out[i][2] as i64).checked_add(columns[3][inherited]).ok_or(())?).map_err(|_|())?;inherited+=1;},
            _=>return Err(())
        }}if fresh!=nf || inherited!=n-nf{return Err(());}
    }else{return Err(());}
    if out.iter().any(|p|p[0]<0 || p[1]<0 || p[0]>641200267 || p[1]>641200267){return Err(());}Ok(out)
}
/// WASMとネイティブで共通の整数API。offsetsは親ごとの点数の累積。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn codec_decode_global_i32(input:*const u8,len:usize,origin_x:i32,origin_y:i32,parent:*const i32,parent_ints:usize,offsets:*const u32,parent_count:usize,output:*mut i32,capacity:usize)->i32 {
    if input.is_null() || output.is_null() || len>16777216 || parent_count>4 || parent_ints>4*262144*3 || parent_ints%3!=0 || capacity>262144{return -1;}
    let bytes=unsafe{slice::from_raw_parts(input,len)};
    let flat=if parent_ints==0 {&[][..]}else{if parent.is_null(){return -1;}unsafe{slice::from_raw_parts(parent,parent_ints)}};
    let off=if parent_count==0 {&[0u32][..]}else{if offsets.is_null(){return -1;}unsafe{slice::from_raw_parts(offsets,parent_count+1)}};
    if off[0]!=0 || off.windows(2).any(|w|w[0]>w[1]) || *off.last().unwrap() as usize!=parent_ints/3{return -1;}
    let p=(0..parent_count).map(|i|flat[off[i] as usize*3..off[i+1] as usize*3].chunks_exact(3).map(|p|[p[0],p[1],p[2]]).collect()).collect::<Vec<_>>();
    match decode_global(bytes,[origin_x,origin_y],&p){Ok(points) if points.len()<=capacity=>{let out=unsafe{slice::from_raw_parts_mut(output,points.len()*3)};for (o,p) in out.chunks_exact_mut(3).zip(&points){o.copy_from_slice(p);}points.len() as i32},_=>-1}
}
#[cfg(test)]
mod tests {
    use super::*;
    fn point() -> Vec<u8> {
        let mut b = b"NQC1".to_vec(); for v in [1_u32, 0, 1, 0.0625_f32.to_bits(), 0.125_f32.to_bits()] { b.extend(v.to_le_bytes()); }
        for value in [16_u8, 32, 80] { let mut payload = vec![0]; let v = value as u16 * 2; if v >= 128 { payload.push((v as u8 & 127) | 128); payload.push((v >> 7) as u8); } else { payload.push(v as u8); } payload.push(0); let mut block = vec![1, payload.len() as u8]; block.extend(payload); b.extend((block.len() as u32).to_le_bytes()); b.extend(block); } b
    }
    #[test] fn independent_point() { let mut out = [0.; 3]; assert_eq!(decode(&point(), &[], &[0], &mut out), Ok(1)); assert_eq!(out, [1., 2., 10.]); }
    #[test] fn truncated_and_trailing() { let mut out = [0.; 3]; let b = point(); for end in 0..b.len() { assert!(decode(&b[..end], &[], &[0], &mut out).is_err()); } let mut extra = b; extra.push(0); assert!(decode(&extra, &[], &[0], &mut out).is_err()); }
    #[test] fn rice_zero_and_run() { assert_eq!(stream(&[3, 4, 2, 0, 3, 7]), Ok(vec![0, 0, 0])); assert_eq!(stream(&[3, 3, 1, 0, 3]), Ok(vec![0, 0, 0])); }
    #[test] fn reject_overflow_and_bad_run() { assert!(stream(&[1, 3, 1, 0, 2]).is_err()); assert!(stream(&[1, 3, 0, 0, 64]).is_err()); assert!(stream(&[1, 5, 2, 0, 3, 0]).is_err()); }
    #[test] fn small_byte_fuzz_never_panics() { for n in 0..256 { for byte in [0, 1, 2, 127, 255] { let mut b = vec![byte; n]; if n >= 4 { b[..4].copy_from_slice(b"NDC1"); } let mut out = [0.; 30]; assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&b, &[], &[0], &mut out))).is_ok()); } } }
}

pub(crate) fn limited_stream(bytes:&[u8],limit:usize)->R<Vec<i64>> {if Reader::new(bytes).var()?>limit as u64{return Err(());}stream(bytes)}
