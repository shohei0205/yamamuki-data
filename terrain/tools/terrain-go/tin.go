// 全国追加用の整数格子TIN。追加順の同点判定もRust版に合わせる。
package main

import (
	"container/heap"
	"fmt"
	"math"
	"sort"
)

type point struct{ X, Y int64 }
type face [3]int
type edge [2]int
type entry struct {
	Score      float64
	ID, Sample int
}
type tri struct {
 Candidates []entry
	F        face
	E        entry
	Absolute float64
}
type triHeap []entry

func (h triHeap) Len() int { return len(h) }
func (h triHeap) Less(i, j int) bool {
	a, b := h[i], h[j]
	if a.Score != b.Score {
		return a.Score > b.Score
	}
	if a.Sample != b.Sample {
		return a.Sample < b.Sample
	}
	return a.ID < b.ID
}
func (h triHeap) Swap(i, j int)  { h[i], h[j] = h[j], h[i] }
func (h *triHeap) Push(v any)    { *h = append(*h, v.(entry)) }
func (h *triHeap) Pop() any      { a := *h; v := a[len(a)-1]; *h = a[:len(a)-1]; return v }
func orient(a, b, c point) int64 { return (b.X-a.X)*(c.Y-a.Y) - (b.Y-a.Y)*(c.X-a.X) }

// Q4～Q12の格子は最大16,385四方。外接円判定の中間値もint64内に収まる。
func incircle(a, b, c, p point) int64 {
	ax, ay := a.X-p.X, a.Y-p.Y
	bx, by := b.X-p.X, b.Y-p.Y
	cx, cy := c.X-p.X, c.Y-p.Y
	return (ax*ax+ay*ay)*(bx*cy-by*cx) - (bx*bx+by*by)*(ax*cy-ay*cx) + (cx*cx+cy*cy)*(ax*by-ay*bx)
}
func edges(f face) [3]edge { return [3]edge{{f[0], f[1]}, {f[1], f[2]}, {f[2], f[0]}} }

type builder struct {
	// 比較試験用。全国追加処理では保存タイルの共有辺を固定する。
	AllowBoundary bool
	CacheCandidates bool
 DensityInactive    bool
	N                  int
	Grid, Inv, Density []float64
	Free, MaxError     float64
	Points             []point
	Tris               map[int]tri
	Edges              map[edge]int
	Selected           []bool
	Blocked            map[int]bool
	Heap               triHeap
	NextID             int
	Samples            uint64
}

func newBuilder(n int, grid, limits []float64, p []point, f []face, counts []float64, free, maxError float64) (*builder, error) {
	if n < 2 || n > 16385 || len(grid) != n*n || len(limits) != n*n || len(counts) != 81 {
		return nil, fmt.Errorf("格子または密度のサイズが不正です")
	}
	b := &builder{N: n, Grid: grid, Inv: make([]float64, len(grid)), Density: append([]float64(nil), counts...), Free: free, MaxError: maxError, Points: append([]point(nil), p...), Tris: map[int]tri{}, Edges: map[edge]int{}, Selected: make([]bool, n*n), Blocked: map[int]bool{}}
	b.CacheCandidates = n<=2049
 b.DensityInactive = free > 0
	for _, v := range counts {
		if v > free {
			b.DensityInactive = false
		}
	}
	for i, v := range limits {
		if !finite(v) || v <= 0 || !finite(grid[i]) {
			return nil, fmt.Errorf("標高または重みが不正です")
		}
		b.Inv[i] = 1 / v
	}
	for _, v := range p {
		if v.X < 0 || v.Y < 0 || v.X >= int64(n) || v.Y >= int64(n) {
			return nil, fmt.Errorf("格子外の頂点です")
		}
		s := int(v.Y)*n + int(v.X)
		if b.Selected[s] {
			return nil, fmt.Errorf("頂点が重複しています")
		}
		b.Selected[s] = true
	}
	var area int64
	for _, v := range f {
		for _, id := range v {
			if id < 0 || id >= len(p) {
				return nil, fmt.Errorf("頂点番号が不正です")
			}
		}
		area += orient(p[v[0]], p[v[1]], p[v[2]])
		if e := b.add(v); e != nil {
			return nil, e
		}
	}
	if area != 2*int64(n-1)*int64(n-1) {
		return nil, fmt.Errorf("メッシュの被覆面積が不正です")
	}
	for e, id := range b.Edges {
		if e[0] > e[1] {
			continue
		}
		other, ok := b.Edges[edge{e[1], e[0]}]
		if !ok {
			continue
		}
		a := b.Tris[id].F
		for _, v := range b.Tris[other].F {
			if v != e[0] && v != e[1] && incircle(p[a[0]], p[a[1]], p[a[2]], p[v]) > 0 {
				return nil, fmt.Errorf("初期TINのDelaunay条件違反")
			}
		}
	}
	return b, nil
}
func finite(v float64) bool { return !math.IsNaN(v) && !math.IsInf(v, 0) }
func (b *builder) count(x, y int) float64 {
	step := float64(b.N-1) / 8
	u, v := float64(x)/step, float64(y)/step
	a, c := int(math.Floor(u)), int(math.Floor(v))
	s, t := u-float64(a), v-float64(c)
	at := func(x, y int) float64 { return b.Density[min(y, 8)*9+min(x, 8)] }
	return at(a, c)*(1-s)*(1-t) + at(a+1, c)*s*(1-t) + at(a, c+1)*(1-s)*t + at(a+1, c+1)*s*t
}
func (b *builder) evaluate(f face) tri {return b.scan(f,true)}
func (b *builder) scan(f face,collect bool) tri {
	a, c, d := b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]
	area := float64(orient(a, c, d))
	za := b.Grid[int(a.Y)*b.N+int(a.X)]
	db := b.Grid[int(c.Y)*b.N+int(c.X)] - za
	dc := b.Grid[int(d.Y)*b.N+int(d.X)] - za
	t := tri{F: f}
	low, high := min(a.Y, c.Y, d.Y), min(max(a.Y, c.Y, d.Y), int64(b.N-1))
	ps := [3]point{a, c, d}
	for y := low; y <= high; y++ {
		left, right := math.Inf(1), math.Inf(-1)
		for k, p := range ps {
			q := ps[(k+1)%3]
			if y < min(p.Y, q.Y) || y > max(p.Y, q.Y) {
				continue
			}
			if p.Y == q.Y {
				left = math.Min(left, float64(min(p.X, q.X)))
				right = math.Max(right, float64(max(p.X, q.X)))
			} else {
				x := float64(p.X) + float64(y-p.Y)*float64(q.X-p.X)/float64(q.Y-p.Y)
				left = math.Min(left, x)
				right = math.Max(right, x)
			}
		}
		if math.IsInf(left, 0) {
			continue
		}
		lo, hi := max(int64(math.Floor(left))-1, 0), min(int64(math.Ceil(right))+1, int64(b.N-1))
		for x := lo; x <= hi; x++ {
			p := point{x, y}
			e0, e1, e2 := orient(a, c, p), orient(c, d, p), orient(d, a, p)
			if e0 < 0 || e1 < 0 || e2 < 0 {
				continue
			}
			sample := int(y)*b.N + int(x)
			if b.Selected[sample] {
				continue
			}
			b.Samples++
			abs := math.Abs((b.Grid[sample] - za) - (float64(e2)/area*db + float64(e0)/area*dc))
			t.Absolute = math.Max(t.Absolute, abs)
            if !collect {continue}
			if (!b.AllowBoundary && (x == 0 || y == 0 || x == int64(b.N-1) || y == int64(b.N-1))) || b.Blocked[sample] {
				continue
			}
			score := abs * b.Inv[sample]
			if !b.DensityInactive {
				score /= 1 + math.Max(b.count(int(x), int(y))-b.Free, 0)/4096
			}
			if b.CacheCandidates && score>0 {t.Candidates=append(t.Candidates,entry{Score:score,Sample:sample})}
            if score > t.E.Score || (score == t.E.Score && sample < t.E.Sample) {
				t.E.Score, t.E.Sample = score, sample
			}
		}
	}
if b.CacheCandidates&&collect {sort.Slice(t.Candidates,func(i,j int)bool{a,c:=t.Candidates[i],t.Candidates[j];if a.Score!=c.Score{return a.Score>c.Score};return a.Sample<c.Sample})}
    return t
}
func (b *builder) add(f face) error {
	if orient(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]) <= 0 {
		return fmt.Errorf("面積ゼロまたは反転面です")
	}
	t := b.evaluate(f)
	id := b.NextID
	b.NextID++
	t.E.ID = id
	for _, e := range edges(f) {
		if _, ok := b.Edges[e]; ok {
			return fmt.Errorf("有向辺が重複しています")
		}
		b.Edges[e] = id
	}
	b.Tris[id] = t
	heap.Push(&b.Heap, t.E)
	return nil
}
func (b *builder) refresh() {
	b.Heap = nil
	ids := b.ids()
	for _, id := range ids {
		t := b.evaluate(b.Tris[id].F)
		t.E.ID = id
		b.Tris[id] = t
		heap.Push(&b.Heap, t.E)
	}
}
func (b *builder) ids() []int {
	a := make([]int, 0, len(b.Tris))
	for id := range b.Tris {
		a = append(a, id)
	}
	sort.Ints(a)
	return a
}
func (b *builder) worst() entry {
	for len(b.Heap) > 0 {
		e := b.Heap[0]
		t, ok := b.Tris[e.ID]
		if !ok || e.Sample != t.E.Sample || e.Score != t.E.Score {
			heap.Pop(&b.Heap)
			continue
		}
		return e
	}
	return entry{}
}
func (b *builder) cavity(e entry) map[int]bool {
	p := point{int64(e.Sample % b.N), int64(e.Sample / b.N)}
	c, v := map[int]bool{}, map[int]bool{}
	stack := []int{e.ID}
	for len(stack) > 0 {
		id := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		if v[id] {
			continue
		}
		v[id] = true
		f := b.Tris[id].F
		if id != e.ID && incircle(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]], p) <= 0 {
			continue
		}
		c[id] = true
		for _, edge := range edges(f) {
			if other, ok := b.Edges[[2]int{edge[1], edge[0]}]; ok {
				stack = append(stack, other)
			}
		}
	}
	return c
}
func (b *builder) boundary(c map[int]bool) []edge {
	out := []edge{}
	for id := range c {
		for _, e := range edges(b.Tris[id].F) {
			other, ok := b.Edges[edge{e[1], e[0]}]
			if !ok || !c[other] {
				out = append(out, e)
			}
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i][0] != out[j][0] {
			return out[i][0] < out[j][0]
		}
		return out[i][1] < out[j][1]
	})
	return out
}
func angle(a, c, d point) float64 {
	p := [3]point{a, c, d}
	out := 180.
	for i, v := range p {
		q, r := p[(i+1)%3], p[(i+2)%3]
		ux, uy, vx, vy := float64(q.X-v.X), float64(q.Y-v.Y), float64(r.X-v.X), float64(r.Y-v.Y)
		cos := (ux*vx + uy*vy) / (math.Hypot(ux, uy) * math.Hypot(vx, vy))
		out = math.Min(out, math.Acos(max(-1, min(1, cos)))*180/math.Pi)
	}
	return out
}
func (b *builder) safe(e entry) bool {
	p := point{int64(e.Sample % b.N), int64(e.Sample / b.N)}
	boundary := b.boundary(b.cavity(e))
	faces := []face{}
	for _, v := range boundary {
		a, c := b.Points[v[0]], b.Points[v[1]]
		if orient(a, c, p) > 0 {
			if angle(a, c, p) < 10-1e-7 {
				return false
			}
			faces = append(faces, face{v[0], v[1], len(b.Points)})
		}
	}
	b.Points = append(b.Points, p)
	defer func() { b.Points = b.Points[:len(b.Points)-1] }()
	for _, f := range faces {
		if b.scan(f,false).Absolute > b.MaxError+1e-6 {
			return false
		}
	}
	return true
}
func (b *builder) next() entry {
	for {
		e := b.worst()
		if e.Score <= .1 || b.safe(e) {
			return e
		}
		b.Blocked[e.Sample] = true
		heap.Pop(&b.Heap)
		t:=b.Tris[e.ID]
        if b.CacheCandidates {
            for len(t.Candidates)>0&&b.Blocked[t.Candidates[0].Sample]{t.Candidates=t.Candidates[1:]}
            t.E=entry{ID:e.ID};if len(t.Candidates)>0 {t.E=t.Candidates[0];t.E.ID=e.ID}
        }else{t=b.evaluate(t.F);t.E.ID=e.ID}
		b.Tris[e.ID] = t
		heap.Push(&b.Heap, t.E)
	}
}
func (b *builder) setDensity(v []float64) {
	same := true
	inactive := b.Free > 0
	for i, x := range v {
		if x != b.Density[i] {
			same = false
		}
		if x > b.Free || b.Density[i] > b.Free {
			inactive = false
		}
	}
	b.Density = append(b.Density[:0], v...)
	b.DensityInactive = b.Free > 0
	for _, x := range v {
		if x > b.Free {
			b.DensityInactive = false
		}
	}
	if !same && !inactive {
		b.refresh()
	}
}
func (b *builder) step(e entry) error {
	p := point{int64(e.Sample % b.N), int64(e.Sample / b.N)}
	if b.Selected[e.Sample] {
		return fmt.Errorf("挿入済みの頂点です")
	}
	c := b.cavity(e)
	boundary := b.boundary(c)
	first := b.NextID
	for id := range c {
		for _, e := range edges(b.Tris[id].F) {
			delete(b.Edges, e)
		}
		delete(b.Tris, id)
	}
	v := len(b.Points)
	b.Points = append(b.Points, p)
	b.Selected[e.Sample] = true
	for _, e := range boundary {
		area := orient(b.Points[e[0]], b.Points[e[1]], p)
		if area < 0 {
			return fmt.Errorf("Delaunay更新領域が不正です")
		}
		if area > 0 {
			if err := b.add(face{e[0], e[1], v}); err != nil {
				return err
			}
		}
	}
	// 接続の変わった範囲内では、以前挿入できなかった点も再評価する。
	lo, hi := point{1 << 60, 1 << 60}, point{-1, -1}
	for id := first; id < b.NextID; id++ {
		for _, v := range b.Tris[id].F {
			q := b.Points[v]
			lo.X = min(lo.X, q.X)
			lo.Y = min(lo.Y, q.Y)
			hi.X = max(hi.X, q.X)
			hi.Y = max(hi.Y, q.Y)
		}
	}
	restored := false
	for sample := range b.Blocked {
		x, y := int64(sample%b.N), int64(sample/b.N)
		if x >= lo.X && x <= hi.X && y >= lo.Y && y <= hi.Y {
			delete(b.Blocked, sample)
			restored = true
		}
	}
	if restored {
		b.refresh()
	}
	if len(b.Heap) > len(b.Tris)*4+64 {
		b.Heap = nil
		for _, id := range b.ids() {
			heap.Push(&b.Heap, b.Tris[id].E)
		}
	}
	return nil
}
