package main

import (
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"testing"
)

// 明示した試験ディレクトリだけを使い、全国生成の入力・出力には触れない。
func TestHierarchyExperiment(t *testing.T) {
	dir := os.Getenv("TERRAIN_HIERARCHY_DIR")
	if dir == "" {
		t.Skip("地域比較を明示した場合だけ生成する")
	}
	for _, region := range []string{"fuji", "yari", "tsukuba"} {
		raw, e := os.ReadFile(filepath.Join(dir, region+".f64"))
		if e != nil {
			t.Fatal(e)
		}
		n := int(math.Sqrt(float64(len(raw) / 8)))
		if n*n*8 != len(raw) {
			t.Fatal("格子サイズが不正")
		}
		grid := make([]float64, n*n)
		limits := make([]float64, n*n)
		for i := range grid {
			grid[i] = math.Float64frombits(le.Uint64(raw[i*8:]))
			limits[i] = 1
		}
		p := []point{{0, 0}, {int64(n - 1), 0}, {int64(n - 1), int64(n - 1)}, {0, int64(n - 1)}}
		b, e := newBuilder(n, grid, limits, p, []face{{0, 1, 2}, {0, 2, 3}}, make([]float64, 81), 4096, 1e9)
		if e != nil {
			t.Fatal(e)
		}
		type stage struct {
			Error    float64
			Points   [][3]float64
			Faces    []face
			IDs      []int
			MaxError float64
			MinAngle float64
		}
		b.AllowBoundary = true
		b.refresh()
		stages := []stage{}
		for _, target := range []float64{800, 400, 200, 100, 50, 25, 12.5} {
			for len(b.Points) < 32768 {
				v := b.next()
				if v.Score <= target {
					break
				}
				if e = b.step(v); e != nil {
					t.Fatal(e)
				}
			}
			s := stage{Error: target, MinAngle: 180}
			for _, v := range b.Points {
				s.Points = append(s.Points, [3]float64{float64(v.X) * 31.25, float64(v.Y) * 31.25, grid[int(v.Y)*n+int(v.X)]})
			}
			var area int64
			for _, id := range b.ids() {
				f := b.Tris[id].F
				s.IDs = append(s.IDs, id)
				s.Faces = append(s.Faces, f)
				s.MaxError = math.Max(s.MaxError, b.scan(f, false).Absolute)
				s.MinAngle = math.Min(s.MinAngle, angle(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]))
				area += orient(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]])
			}
			if area != 2*int64(n-1)*int64(n-1) {
				t.Fatal("被覆面積の不一致")
			}
			for ed, id := range b.Edges {
				if other, ok := b.Edges[edge{ed[1], ed[0]}]; ok {
					f := b.Tris[id].F
					for _, v := range b.Tris[other].F {
						if v != ed[0] && v != ed[1] && incircle(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]], b.Points[v]) > 0 {
							t.Fatal("Delaunay条件違反")
						}
					}
				}
			}
			if len(stages) > 0 {
				prev := stages[len(stages)-1]
				for i, v := range prev.Points {
					if s.Points[i] != v {
						t.Fatal("親頂点が変わりました")
					}
				}
			}
			stages = append(stages, s)
			t.Log(region, target, len(s.Points), s.MaxError, s.MinAngle)
		}
		data, e := json.Marshal(stages)
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(dir, region+".json"), data, 0644); e != nil {
			t.Fatal(e)
		}
	}
}

func TestHierarchyBoundaryRefinement(t *testing.T) {
	n := 17
	g := make([]float64, n*n)
	limits := make([]float64, n*n)
	for y := 0; y < n; y++ {
		for x := 0; x < n; x++ {
			g[y*n+x] = 100 * math.Sin(float64(x)*math.Pi/16)
			limits[y*n+x] = 1
		}
	}
	b, e := newBuilder(n, g, limits, []point{{0, 0}, {16, 0}, {16, 16}, {0, 16}}, []face{{0, 1, 2}, {0, 2, 3}}, make([]float64, 81), 4096, 1e9)
	if e != nil {
		t.Fatal(e)
	}
	b.AllowBoundary = true
	b.refresh()
	for len(b.Points) < 100 {
		v := b.next()
		if v.Score < 5 {
			break
		}
		if e = b.step(v); e != nil {
			t.Fatal(e)
		}
	}
	found := false
	for _, p := range b.Points[4:] {
		if p.Y == 0 || p.Y == 16 {
			found = true
		}
	}
	if !found {
		t.Fatal("外周の頂点が追加されませんでした")
	}
	var area int64
	for _, f := range b.Tris {
		area += orient(b.Points[f.F[0]], b.Points[f.F[1]], b.Points[f.F[2]])
	}
	if area != 512 {
		t.Fatal("外周挿入で被覆が変わりました")
	}
}
