"""汎用圧縮を使わず、整数ブロックと親参照を直接符号化する比較形式。"""
import struct
import numpy as np

HEADER=struct.Struct('<4sIIIff')
def varint(value):
 value=int(value)
 if value<0:raise ValueError('負の符号なし整数')
 out=bytearray()
 while value>=128:out.append((value&127)|128);value>>=7
 out.append(value);return bytes(out)
def readvar(data,pos):
 value=0
 for shift in range(0,70,7):
  if pos>=len(data):raise ValueError('可変長整数の途中で終了')
  byte=data[pos];pos+=1;value|=(byte&127)<<shift
  if byte<128:return value,pos
 raise ValueError('可変長整数が長すぎます')
def zigzag(v):return np.where(v>=0,v*2,-v*2-1).astype(np.uint64)
def unzigzag(v):
 v=np.asarray(v,np.int64);return np.where(v&1,-(v//2)-1,v//2)
def bits(values,width):
 if not width:return b''
 return np.packbits(((np.asarray(values,np.uint64)[:,None]>>np.arange(width,dtype=np.uint64))&1).astype(np.uint8).ravel(),bitorder='little').tobytes()
def unbits(data,count,width):
 if not width:return np.zeros(count,np.int64)
 if len(data)!=(count*width+7)//8:raise ValueError('ビット配列長が不正')
 b=np.unpackbits(np.frombuffer(data,np.uint8),bitorder='little')[:count*width].reshape(count,width);return (b.astype(np.int64)*(1<<np.arange(width,dtype=np.int64))).sum(axis=1)

def encode_stream(values,rice=True):
 values=np.asarray(values,np.int64)
 if np.any((values<-(2**62)+1)|(values>2**62-1)):raise ValueError('整数の符号化範囲外')
 out=bytearray(varint(len(values)))
 for start in range(0,len(values),128):
  v=values[start:start+128];base=int(v.min());residual=v-base;width=int(residual.max()).bit_length();candidates=[b'\x00'+varint(int(zigzag(np.array([base]))[0]))+bytes([width])+bits(residual,width)]
  # 0の連続をまとめ、非0をzigzag+1で表す。
  z=zigzag(v);encoded=bytearray(b'\x01');i=0
  while i<len(z):
   if z[i]==0:
    j=i+1
    while j<len(z) and z[j]==0:j+=1
    encoded.extend(b'\0'+varint(j-i));i=j
   else:encoded.extend(varint(int(z[i])+1));i+=1
  candidates.append(bytes(encoded))
  if rice:
   mean=sum(map(int,z))//len(z);guess=max(0,mean.bit_length()-1);
   for k in range(max(0,guess-1),min(32,guess+1)+1):
    q=z>>k;length=int(q.sum())+len(v)*(k+1)
    if length>min(map(len,candidates))*8:continue
    b=np.ones(length,np.uint8);cursor=0
    for value,quotient in zip(z,q):
     quotient=int(quotient);b[cursor:cursor+quotient]=0;cursor+=quotient;b[cursor]=1;cursor+=1
     for bit in range(k):b[cursor]=(int(value)>>bit)&1;cursor+=1
    candidates.append(b'\x02'+bytes([k])+varint(length)+np.packbits(b,bitorder='little').tobytes())
  chosen=min(candidates,key=len);out.extend(varint(len(chosen)));out.extend(chosen)
 return bytes(out)

def decode_stream(blob):
 data=memoryview(blob);count,pos=readvar(data,0)
 if count>10000000:raise ValueError('整数列が長すぎます')
 result=[]
 while len(result)<count:
  length,pos=readvar(data,pos);end=pos+length
  if not length or end>len(data):raise ValueError('ブロック長が不正')
  b=data[pos:end];pos=end;n=min(128,count-len(result));kind=b[0];p=1
  if kind==0:
   base,p=readvar(b,p)
   if p>=len(b):raise ValueError('ビット幅がありません')
   width=b[p];p+=1
   if width>63:raise ValueError('ビット幅が大きすぎます')
   v=unbits(b[p:],n,width)+int(unzigzag(np.array([base]))[0]);result.extend(v.tolist())
  elif kind==1:
   z=[]
   while len(z)<n:
    value,p=readvar(b,p)
    if value:z.append(value-1)
    else:
     run,p=readvar(b,p)
     if not 0<run<=n-len(z):raise ValueError('0の連続長が不正')
     z.extend([0]*run)
   if p!=len(b):raise ValueError('余分な可変長データ')
   result.extend(unzigzag(np.asarray(z)).tolist())
  elif kind==2:
   k=b[p];p+=1;length,p=readvar(b,p)
   if k>32 or len(b)-p!=(length+7)//8:raise ValueError('Riceブロックが不正')
   bit=np.unpackbits(np.frombuffer(b[p:],np.uint8),bitorder='little')[:length];cursor=0;z=[]
   for _ in range(n):
    quotient=0
    while cursor<len(bit) and bit[cursor]==0:quotient+=1;cursor+=1
    if cursor+k>=len(bit):raise ValueError('Rice符号の途中で終了')
    cursor+=1;remainder=0
    for j in range(k):remainder|=int(bit[cursor])<<j;cursor+=1
    z.append((quotient<<k)|remainder)
   if cursor!=len(bit):raise ValueError('余分なRice符号')
   result.extend(unzigzag(np.asarray(z)).tolist())
  else:raise ValueError('未知の整数ブロック')
 if pos!=len(data):raise ValueError('余分な整数列')
 return np.asarray(result,np.int64)

def pack_streams(magic,n,parents,fresh,streams,xy=.0625,z=.125,rice=True):
 encoded=[encode_stream(v,rice=rice) for v in streams];return HEADER.pack(magic,n,parents,fresh,xy,z)+b''.join(struct.pack('<I',len(b))+b for b in encoded)
def unpack_streams(blob):
 if len(blob)<HEADER.size:raise ValueError('ヘッダー不足')
 magic,n,parents,fresh,xy,z=HEADER.unpack_from(blob);pos=HEADER.size;streams=[]
 if magic not in [b'NQC1',b'NDC1'] or n>10000000 or fresh>n or parents>256 or not np.isfinite([xy,z]).all() or min(xy,z)<=0:raise ValueError('不正なメッシュ形式')
 while pos<len(blob):
  if pos+4>len(blob):raise ValueError('列長がありません')
  size=struct.unpack_from('<I',blob,pos)[0];pos+=4
  if pos+size>len(blob):raise ValueError('列が途中で終了')
  streams.append(decode_stream(blob[pos:pos+size]));pos+=size
 return (magic,n,parents,fresh,xy,z),streams
def encode_independent(v,xy=.0625,z=.125,rice=True):
 v=np.asarray(v,np.int64);d=np.diff(v,axis=0,prepend=np.zeros((1,3),np.int64));return pack_streams(b'NQC1',len(v),0,len(v),d.T,xy,z,rice)
def decode_independent(blob):
 h,s=unpack_streams(blob)
 if h[0]!=b'NQC1' or h[2]!=0 or h[3]!=h[1] or len(s)!=3 or any(len(v)!=h[1] for v in s):raise ValueError('独立点列の長さが不正')
 return np.cumsum(np.column_stack(s),axis=0)
def encode_delta(refs,delta,fresh,parents,rice=True):
 refs=np.asarray(refs,np.uint32);mask=refs==0xffffffff;inherited=refs[~mask];slots=(inherited>>24).astype(np.int64);indices=(inherited&0xffffff).astype(np.int64);predicted=np.zeros(len(indices),np.int64)
 for slot in range(parents):
  ids=np.flatnonzero(slots==slot);predicted[ids]=np.diff(indices[ids],prepend=0)
 if np.any(slots>=parents):raise ValueError('親の範囲外')
 f=np.asarray(fresh,np.int64).reshape(-1,3);d=np.diff(f,axis=0,prepend=np.zeros((1,3),np.int64));return pack_streams(b'NDC1',len(refs),parents,len(f),[mask.astype(np.int64),slots,predicted,np.asarray(delta)[~mask],*d.T],rice=rice)
def decode_delta_arrays(blob):
 h,s=unpack_streams(blob);_,n,parents,nf,_,_=h
 if h[0]!=b'NDC1' or len(s)!=7 or len(s[0])!=n or not np.isin(s[0],[0,1]).all():raise ValueError('差分の型配列が不正')
 mask=s[0].astype(bool);slots,d,heights=s[1:4];ni=n-nf
 if mask.sum()!=nf or any(len(v)!=ni for v in s[1:4]) or any(len(v)!=nf for v in s[4:]) or np.any((slots<0)|(slots>=parents)):raise ValueError('差分列の長さ・参照が不正')
 indices=np.zeros(ni,np.int64)
 for slot in range(parents):
  ids=np.flatnonzero(slots==slot);indices[ids]=np.cumsum(d[ids])
 if np.any((indices<0)|(indices>=2**24)):raise ValueError('親の頂点番号が範囲外')
 refs=np.full(n,0xffffffff,np.uint32);refs[~mask]=((slots<<24)|indices).astype(np.uint32);delta=np.zeros(n,np.int64);delta[~mask]=heights;fresh=np.cumsum(np.column_stack(s[4:]),axis=0);return refs,delta,fresh
def decode_delta(blob,parents):
 refs,delta,fresh=decode_delta_arrays(blob);h=HEADER.unpack_from(blob)
 if len(parents)!=h[2]:raise ValueError('親数が不一致')
 mask=refs==0xffffffff;out=np.empty((len(refs),3),np.float32);out[mask]=fresh*[h[4],h[4],h[5]]
 for slot,parent in enumerate(parents):
  ids=np.flatnonzero((~mask)&((refs>>24)==slot));index=refs[ids]&0xffffff
  if len(index) and index.max()>=len(parent):raise ValueError('親番号が範囲外')
  out[ids]=parent[index];out[ids,2]+=delta[ids]*h[5]
 return out
def decode_points(blob,parents=()):
 h=HEADER.unpack_from(blob)
 if h[0]==b'NQC1':
  if parents:raise ValueError('独立データに親はありません')
  return (decode_independent(blob)*[h[4],h[4],h[5]]).astype(np.float32)
 return decode_delta(blob,parents)
