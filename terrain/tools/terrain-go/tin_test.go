package main

import (
	"bytes"
 "compress/gzip"
 "encoding/binary"
 "github.com/ulikunitz/xz"
	"math"
	"path/filepath"
	"testing"
)

func testBuilder(t *testing.T, free float64) *builder {
	n := 17
	z := make([]float64, n*n)
	limits := make([]float64, n*n)
	for i := range z {
		z[i] = float64((i*97)%101) / 10
		limits[i] = 25
	}
	b, e := newBuilder(n, z, limits, []point{{0, 0}, {16, 0}, {16, 16}, {0, 16}}, []face{{0, 1, 2}, {0, 2, 3}}, make([]float64, 81), free, 25)
	if e != nil {
		t.Fatal(e)
	}
	return b
}
func TestIntegerPredicates(t *testing.T) {
	a, b, c := point{0, 0}, point{16384, 0}, point{16384, 16384}
	if orient(a, b, c) <= 0 || incircle(a, b, c, point{8192, 8192}) <= 0 || incircle(a, b, c, point{0, 16384}) != 0 {
		t.Fatal("整数幾何判定")
	}
}
func TestInsertionKeepsDelaunayAndCoverage(t *testing.T) {
	b := testBuilder(t, 4096)
	for i := 0; i < 30; i++ {
		e := b.next()
		if e.Score <= .1 {
			break
		}
		if err := b.step(e); err != nil {
			t.Fatal(err)
		}
		var area int64
		for _, v := range b.Tris {
			f := v.F
			a, c, d := b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]
			o := orient(a, c, d)
			if o <= 0 || angle(a, c, d) < 10-1e-7 {
				t.Fatal("面品質")
			}
			area += o
			for _, p := range b.Points {
				if incircle(a, c, d, p) > 0 {
					t.Fatal("Delaunay違反")
				}
			}
		}
		if area != 512 {
			t.Fatal("被覆面積")
		}
	}
}
func TestDensityFreeRange(t *testing.T) {
	b := testBuilder(t, 4096)
	base := b.next()
	v := make([]float64, 81)
	for i := range v {
		v[i] = 4096
	}
	b.setDensity(v)
	if !b.DensityInactive || b.next() != base {
		t.Fatal("無減点範囲")
	}
	for i := range v {
		v[i] = 8192
	}
	b.setDensity(v)
	got := b.next()
	if b.DensityInactive || math.Abs(got.Score-base.Score/2) > 1e-12 {
		t.Fatal("超過時の減点")
	}
}
func TestDensityAcrossBorders(t *testing.T) {
	d := newDensity(job{Q: 4})
	d.add(point{64, 32})
	a, b := d.tile(key{}), d.tile(key{X: 1})
	if a[4*9+8] != b[4*9] {
		t.Fatal("境界の密度不一致")
	}
}
func TestBadMeshRejected(t *testing.T) {
	for _, raw := range [][]byte{nil, []byte("YQB1"), make([]byte, 12)} {
		if _, _, e := decodeMesh(raw, 65); e == nil {
			t.Fatal("不正なメッシュを受理")
		}
	}
}
func TestRawRoundTrip(t *testing.T) {
	b := testBuilder(t, 0)
	raw := b.raw()
	p, f, e := decodeMesh(raw, b.N)
	if e != nil || len(p) != len(b.Points) || len(f) != len(b.Tris) {
		t.Fatal("形式の復元")
	}
	z, e := zipBytes(raw, 6)
	if e != nil {
		t.Fatal(e)
	}
	r, e := unzip(z)
	if e != nil || !bytes.Equal(r, raw) {
		t.Fatal("可逆圧縮")
	}
}
func TestReadOnlyAndWindowsURI(t *testing.T) {
	p := filepath.Join(t.TempDir(), "標高.sqlite3")
	d, e := openDB(p, false)
	if e != nil {
		t.Fatal(e)
	}
	d.Exec("CREATE TABLE values_test(x)")
	d.Close()
	d, e = openDB(p, true)
	if e != nil {
		t.Fatal(e)
	}
	defer d.Close()
	if _, e = d.Exec("INSERT INTO values_test VALUES(1)"); e == nil {
		t.Fatal("読取専用を破った")
	}
}
func TestExclusiveOutputLock(t *testing.T) {
	p := filepath.Join(t.TempDir(), "生成.lock")
	unlock, e := lockOutput(p)
	if e != nil {
		t.Fatal(e)
	}
	if other, e := lockOutput(p); e == nil {
		other()
		unlock()
		t.Fatal("二重起動を許可")
	}
	unlock()
	unlock, e = lockOutput(p)
	if e != nil {
		t.Fatal(e)
	}
	unlock()
}
func TestManifestRejectsOtherSource(t *testing.T) {
	a := map[string]any{"source": "a", "sourceSize": float64(3), "sourceMtimeNs": float64(4), "levels": []any{map[string]any{"level": float64(4)}}}
	b := map[string]any{"source": "b", "sourceSize": float64(3), "sourceMtimeNs": float64(4), "levels": []any{map[string]any{"level": float64(4)}}}
	if sameFieldSource(a, b) {
		t.Fatal("別の元標高を受理")
	}
}

func TestCandidateCacheMatchesRescan(t *testing.T){
    for _,free:=range []float64{0,4096} {
        a,b:=testBuilder(t,free),testBuilder(t,free);b.CacheCandidates=false;b.refresh()
        for i:=0;i<40;i++ {x,y:=a.next(),b.next();if x!=y{t.Fatalf("追加%d: %+v != %+v",i,x,y)};if x.Score<=.1{break};if e:=a.step(x);e!=nil{t.Fatal(e)};if e:=b.step(y);e!=nil{t.Fatal(e)}}
        if !bytes.Equal(a.raw(),b.raw()){t.Fatal("候補キャッシュによって形状が変化")}
    }
}

func TestSourceGridGzipAndXZ(t *testing.T){
    raw:=make([]byte,256*256*2+8192)
    for y:=0;y<256;y++{for x:=0;x<256;x++{delta:=int16(0);if y==0&&x>0{delta=2};if y>0&&x==0{delta=1};le.PutUint16(raw[(y*256+x)*2:],uint16(delta))}}
    raw[256*256*2+100/8]|=1<<uint(100%8)
    for codec:=0;codec<2;codec++{var payload bytes.Buffer;if codec==0{w:=gzip.NewWriter(&payload);w.Write(raw);w.Close()}else{w,e:=xz.NewWriter(&payload);if e!=nil{t.Fatal(e)};w.Write(raw);w.Close()}
        var header bytes.Buffer;header.WriteString("YMD1");binary.Write(&header,le,uint16(256));binary.Write(&header,le,uint16(256));binary.Write(&header,le,float32(.25));binary.Write(&header,le,int32(400));header.WriteByte(2);header.WriteByte(byte(codec));header.Write(payload.Bytes());values,e:=decodeGrid(header.Bytes());if e!=nil{t.Fatal(e)};if values[0]!=100||values[257]!=100.75||!math.IsNaN(float64(values[100])){t.Fatal("元標高の復元値が不正")}
    }
}

func TestBatchedBasicDensityMatchesPointUpdates(t *testing.T){
    for q:=4;q<=12;q++{a,b:=newDensity(job{Q:q}),newDensity(job{Q:q});sharedA,sharedB:=map[point]bool{},map[point]bool{};var countA,countB uint64
        for _,k:=range []key{{-2,-3},{-1,-3},{-2,-2}}{p:=[]point{{0,0},{a.Stride,0},{a.Stride,a.Stride},{0,a.Stride}};for i:=int64(0);i<100;i++{p=append(p,point{(i*37+3)%(a.Stride+1),(i*97+5)%(a.Stride+1)})}
            for _,v:=range p{world:=point{k.X*a.Stride+v.X,k.Y*a.Stride+v.Y};edge:=v.X==0||v.Y==0||v.X==a.Stride||v.Y==a.Stride;if !edge||!sharedA[world]{a.add(world);countA++;if edge{sharedA[world]=true}}};countB+=b.addBasicTile(k,p,sharedB)
        };if countA!=countB||len(a.Counts)!=len(b.Counts){t.Fatal("基本密度の点数不一致")};for k,v:=range a.Counts{if b.Counts[k]!=v{t.Fatal("基本密度の集計値不一致")}}
    }
}
