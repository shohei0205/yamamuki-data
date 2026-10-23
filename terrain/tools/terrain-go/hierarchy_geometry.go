package main

import (
	"fmt"
	"math"
	"sort"
)

// 標高格子を展開せず、既存頂点を残してDelaunayへ挿入する。
type hierarchyGeometry struct {
	B        *builder
	Selected map[point]int
	Last     int
}

func hierarchySeed(n int, p []point, f []face) (*hierarchyGeometry, error) {
	b := &builder{N: n, Points: append([]point(nil), p...), Tris: map[int]tri{}, Edges: map[edge]int{}}
	g := &hierarchyGeometry{b, map[point]int{}, 0}
	for i, v := range p {
		if _, ok := g.Selected[v]; ok {
			return nil, fmt.Errorf("初期頂点が重複")
		}
		g.Selected[v] = i
	}
	for _, v := range f {
		if e := g.add(v); e != nil {
			return nil, e
		}
	}
	return g, nil
}
func (g *hierarchyGeometry) add(f face) error {
	b := g.B
	if orient(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]) <= 0 {
		return fmt.Errorf("階層TINが反転")
	}
	if b.Grid != nil {
		id := b.NextID
		if e := b.add(f); e != nil {
			return e
		}
		g.Last = id
		return nil
	}
	id := b.NextID
	b.NextID++
	for _, e := range edges(f) {
		if _, ok := b.Edges[e]; ok {
			return fmt.Errorf("階層TINの辺が重複")
		}
		b.Edges[e] = id
	}
	b.Tris[id] = tri{F: f}
	g.Last = id
	return nil
}
func (g *hierarchyGeometry) locate(p point) (int, error) {
	b := g.B
	id := g.Last
	if _, ok := b.Tris[id]; !ok {
		ids := b.ids()
		if len(ids) == 0 {
			return 0, fmt.Errorf("空のTIN")
		}
		id = ids[0]
	}
	for step := 0; step <= len(b.Tris); step++ {
		f, ok := b.Tris[id]
		if !ok {
			break
		}
		next := -1
		for _, ed := range edges(f.F) {
			if orient(b.Points[ed[0]], b.Points[ed[1]], p) < 0 {
				if v, ok := b.Edges[edge{ed[1], ed[0]}]; ok {
					next = v
					break
				}
				return 0, fmt.Errorf("階層頂点がタイル外")
			}
		}
		if next < 0 {
			return id, nil
		}
		id = next
	}
	return 0, fmt.Errorf("階層TINの点探索が循環")
}
func (g *hierarchyGeometry) insert(p point) (int, error) {
	if id, ok := g.Selected[p]; ok {
		return id, nil
	}
	b := g.B
	if p.X < 0 || p.Y < 0 || p.X >= int64(b.N) || p.Y >= int64(b.N) {
		return 0, fmt.Errorf("階層頂点が格子外")
	}
	id, e := g.locate(p)
	if e != nil {
		return 0, e
	}
	cavity := b.cavity(entry{ID: id, Sample: int(p.Y)*b.N + int(p.X)})
	boundary := b.boundary(cavity)
	for id := range cavity {
		for _, ed := range edges(b.Tris[id].F) {
			delete(b.Edges, ed)
		}
		delete(b.Tris, id)
	}
	v := len(b.Points)
	b.Points = append(b.Points, p)
	g.Selected[p] = v
	if b.Selected != nil {
		b.Selected[int(p.Y)*b.N+int(p.X)] = true
	}
	for _, ed := range boundary {
		a := orient(b.Points[ed[0]], b.Points[ed[1]], p)
		if a < 0 {
			return 0, fmt.Errorf("階層TINの更新域が反転")
		}
		if a > 0 {
			if e = g.add(face{ed[0], ed[1], v}); e != nil {
				return 0, e
			}
		}
	}
	return v, nil
}
func (g *hierarchyGeometry) faces() []face {
	out := []face{}
	for _, id := range g.B.ids() {
		out = append(out, g.B.Tris[id].F)
	}
	return out
}
func (g *hierarchyGeometry) minimumAngle() float64 {
	out := 180.
	for _, t := range g.B.Tris {
		out = math.Min(out, angle(g.B.Points[t.F[0]], g.B.Points[t.F[1]], g.B.Points[t.F[2]]))
	}
	return out
}
func (g *hierarchyGeometry) refineAngles(stop func() bool) ([]point, error) {
	added := []point{}
	b := g.B
	for {
		if stop != nil && stop() {
			return added, fmt.Errorf("階層品質補修を保存前に停止")
		}
		worst := -1
		minimum := 10. - 1e-7
		for _, id := range b.ids() {
			f := b.Tris[id].F
			a := angle(b.Points[f[0]], b.Points[f[1]], b.Points[f[2]])
			if a < minimum {
				minimum = a
				worst = id
			}
		}
		if worst < 0 {
			return added, nil
		}
		if len(b.Points) >= 65535 {
			return added, fmt.Errorf("角度補修でuint16頂点上限に到達")
		}
		f := b.Tris[worst].F
		a, c, d := b.Points[f[0]], b.Points[f[1]], b.Points[f[2]]
		ax, ay, cx, cy, dx, dy := float64(a.X), float64(a.Y), float64(c.X), float64(c.Y), float64(d.X), float64(d.Y)
		den := 2 * (ax*(cy-dy) + cx*(dy-ay) + dx*(ay-cy))
		aa, cc, dd := ax*ax+ay*ay, cx*cx+cy*cy, dx*dx+dy*dy
		candidate := point{int64(math.Round((aa*(cy-dy) + cc*(dy-ay) + dd*(ay-cy)) / den)), int64(math.Round((aa*(dx-cx) + cc*(ax-dx) + dd*(cx-ax)) / den))}
		// 外周の線分を侵食するときはその辺から二分する。
		boundary := []edge{}
		for ed := range b.Edges {
			if _, ok := b.Edges[edge{ed[1], ed[0]}]; !ok {
				boundary = append(boundary, ed)
			}
		}
		sort.Slice(boundary, func(i, j int) bool {
			if boundary[i][0] != boundary[j][0] {
				return boundary[i][0] < boundary[j][0]
			}
			return boundary[i][1] < boundary[j][1]
		})
		for _, ed := range boundary {
			p, q := b.Points[ed[0]], b.Points[ed[1]]
			px, py := float64(candidate.X), float64(candidate.Y)
			if (px-float64(p.X))*(px-float64(q.X))+(py-float64(p.Y))*(py-float64(q.Y)) < 0 {
				m := point{(p.X + q.X) / 2, (p.Y + q.Y) / 2}
				if _, exists := g.Selected[m]; !exists {
					candidate = m
					break
				}
			}
		}
		choices := []point{candidate}
		type pair struct {
			P      point
			Length int64
		}
		mids := []pair{}
		for _, ed := range edges(f) {
			p, q := b.Points[ed[0]], b.Points[ed[1]]
			mids = append(mids, pair{point{(p.X + q.X) / 2, (p.Y + q.Y) / 2}, (p.X-q.X)*(p.X-q.X) + (p.Y-q.Y)*(p.Y-q.Y)})
		}
		sort.SliceStable(mids, func(i, j int) bool { return mids[i].Length > mids[j].Length })
		for _, m := range mids {
			choices = append(choices, m.P)
		}
		choices = append(choices, point{(a.X + c.X + d.X) / 3, (a.Y + c.Y + d.Y) / 3})
		found := false
		for _, p := range choices {
			if p.X < 0 || p.Y < 0 || p.X >= int64(b.N) || p.Y >= int64(b.N) {
				continue
			}
			if _, ok := g.Selected[p]; ok {
				continue
			}
			if _, e := g.insert(p); e != nil {
				return added, e
			}
			added = append(added, p)
			found = true
			break
		}
		if !found {
			return added, fmt.Errorf("格子上で最小角10度を満たす補修点を選べません (%.5f度)", minimum)
		}
	}
}
