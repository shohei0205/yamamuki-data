package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"sort"
	"testing"
)
// 実データを明示した場合だけ、小さな目録で親→子の保存・復元を通して検証する。
func TestHierarchyRegionalPipeline(t *testing.T){
 input:=os.Getenv("TERRAIN_HIERARCHY_SMOKE_INPUT");folder:=os.Getenv("TERRAIN_HIERARCHY_SMOKE_DIR");cache:=os.Getenv("TERRAIN_HIERARCHY_SMOKE_CACHE")
 if input==""||folder==""||cache==""{t.Skip("実データ試験の入力・出力先を指定した場合だけ実行")}
 if e:=os.MkdirAll(folder,0700);e!=nil{t.Fatal(e)}
 before,e:=os.ReadFile(input);if e!=nil{t.Fatal(e)}
 o:=options{Hierarchical:true,Base:input,HierarchyOutput:filepath.Join(folder,"basic.sqlite3"),Output:filepath.Join(folder,"pool.sqlite3"),FinalOutput:filepath.Join(folder,"delta.sqlite3"),Cache:cache,Limit:-1,MinQ:6,MaxQ:8,Workers:4,CacheGiB:8,Free:4096,MinFreeGiB:8}
 if e=executeHierarchy(o);e!=nil{t.Fatal(e)}
 after,e:=os.ReadFile(input);if e!=nil||!bytes.Equal(before,after){t.Fatal("試験入力が変化",e)}
 db,e:=openDB(o.FinalOutput,true);if e!=nil{t.Fatal(e)};defer db.Close();var count int;if e=db.QueryRow("SELECT count(*) FROM tiles").Scan(&count);e!=nil||count!=63{t.Fatal("地域タイル数",count,e)}
 rows,e:=db.Query("SELECT q,x,y,triangles,indices,mesh_sha256 FROM tiles ORDER BY q DESC,x,y");if e!=nil{t.Fatal(e)};defer rows.Close()
 for rows.Next(){var q,nf int;var k key;var encoded []byte;var expected string;if e=rows.Scan(&q,&k.X,&k.Y,&nf,&encoded,&expected);e!=nil{t.Fatal(e)};p,e:=restoreHierarchyVertices(db,q,k);if e!=nil{t.Fatal(e)};f,e:=decodeTFC(encoded,nf,len(p));if e!=nil{t.Fatal(e)};if hierarchyDigest(p,f)!=expected{t.Fatal("保存後のハッシュ不一致")};var area int64;for _,v:=range f{area+=orient(p[v[0]].P,p[v[1]].P,p[v[2]].P)};stride:=int64(job{Q:q}.n()-1);if area!=2*stride*stride{t.Fatal("復元後の被覆面積")}}
 if e=rows.Err();e!=nil{t.Fatal(e)};t.Log("全63タイルの保存・復元を確認",folder)
}

func sortedFaces(f []face) []face {
	a := append([]face(nil), f...)
	for i, v := range a {
		j := 0
		if v[1] < v[j] {
			j = 1
		}
		if v[2] < v[j] {
			j = 2
		}
		a[i] = face{v[j], v[(j+1)%3], v[(j+2)%3]}
	}
	sort.Slice(a, func(i, j int) bool {
		for k := 0; k < 3; k++ {
			if a[i][k] != a[j][k] {
				return a[i][k] < a[j][k]
			}
		}
		return false
	})
	return a
}
func TestHierarchyInsertionAndRefinement(t *testing.T) {
	n := 33
	grid := make([]float64, n*n)
	for y := 0; y < n; y++ {
		for x := 0; x < n; x++ {
			grid[y*n+x] = math.Round(400*math.Exp(-float64((x-16)*(x-16)+(y-16)*(y-16))/100)*4) / 4
		}
	}
	points := []point{{0, 0}, {32, 0}, {32, 32}, {0, 32}}
	g, e := hierarchySeed(n, points, []face{{0, 1, 2}, {0, 2, 3}})
	if e != nil {
		t.Fatal(e)
	}
	for _, p := range []point{{16, 16}, {17, 16}, {8, 0}, {0, 8}, {32, 8}, {8, 32}} {
		if _, e = g.insert(p); e != nil {
			t.Fatal(e)
		}
	}
	inherited := append([]point(nil), g.B.Points...)
	zs := map[point]float64{}
	for _, p := range inherited {
		zs[p] = grid[int(p.Y)*n+int(p.X)]
	}
	maximum, e := refineHierarchy(g, grid, zs, 10, nil)
	if e != nil {
		t.Fatal(e)
	}
	if maximum > 10.000001 || g.minimumAngle() < 10-1e-6 {
		t.Fatal(maximum, g.minimumAngle())
	}
	for i, p := range inherited {
		if p != g.B.Points[i] || zs[p] != g.B.Grid[int(p.Y)*n+int(p.X)] {
			t.Fatal("親頂点が変化")
		}
	}
	var area int64
	for _, f := range g.faces() {
		area += orient(g.B.Points[f[0]], g.B.Points[f[1]], g.B.Points[f[2]])
	}
	if area != 2*32*32 {
		t.Fatal("被覆不一致")
	}
}
func TestTFCNativeRoundtrip(t *testing.T) {
	for _, f := range [][]face{{}, {{0, 1, 2}}, {{0, 1, 2}, {2, 1, 3}, {4, 5, 6}}, {{65533, 65534, 65535}}} {
		b, e := encodeTFC(f)
		if e != nil {
			t.Fatal(e)
		}
		decoded, e := decodeTFC(b, len(f), 65536)
		if e != nil {
			t.Fatal(e)
		}
		a, c := sortedFaces(f), sortedFaces(decoded)
		if len(a) != len(c) {
			t.Fatal("面数")
		}
		for i := range a {
			if a[i] != c[i] {
				t.Fatal("面が不一致")
			}
		}
		for i := 0; i < len(b); i++ {
			if _, e = decodeTFC(b[:i], len(f), 65536); e == nil {
				t.Fatal("切断を受理")
			}
		}
	}
}
func TestTFCNativeJSReference(t *testing.T) {
	path := os.Getenv("TERRAIN_HIERARCHY_TFC_REFERENCE")
	if path == "" {
		t.Skip("地域参照を指定した場合だけ実行")
	}
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal(e)
	}
	var v struct {
		Faces []face
		Data  string
	}
	if e = json.Unmarshal(raw, &v); e != nil {
		t.Fatal(e)
	}
	expected, e := base64.StdEncoding.DecodeString(v.Data)
	if e != nil {
		t.Fatal(e)
	}
	actual, e := encodeTFC(v.Faces)
	if e != nil {
		t.Fatal(e)
	}
	if !bytes.Equal(expected, actual) {
		t.Fatal("JSエンコーダーとバイト不一致")
	}
}
