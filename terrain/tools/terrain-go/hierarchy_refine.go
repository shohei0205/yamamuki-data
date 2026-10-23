package main

import (
	"container/heap"
	"fmt"
	"math"
)

// 親頂点を動かさず、辺の切断をせずに品質と標高誤差を補修する。
func refineHierarchy(g *hierarchyGeometry, grid []float64, heights map[point]float64, tolerance float64, stop func() bool) (float64, error) {
	b := g.B
	if len(grid) != b.N*b.N {
		return 0, fmt.Errorf("階層標高のサイズ不正")
	}
	b.Grid = grid
	b.Inv = make([]float64, len(grid))
	b.Selected = make([]bool, len(grid))
	b.Density = make([]float64, 81)
	b.DensityInactive = true
	b.AllowBoundary = true
	b.CacheCandidates = false
	b.Blocked = map[int]bool{}
	for i := range b.Inv {
		b.Inv[i] = 1
	}
	selectedError := 0.
	for _, p := range b.Points {
		i := int(p.Y)*b.N + int(p.X)
		b.Selected[i] = true
		if z, ok := heights[p]; ok {
			d := math.Abs(z - grid[i])
			if d > .250001 {
				return 0, fmt.Errorf("親と子の元標高が不一致 %.3f m", d)
			}
			selectedError = math.Max(selectedError, d)
			grid[i] = z
		}
	}
	b.refresh()
	for {
		if _, e := g.refineAngles(stop); e != nil {
			return 0, e
		}
		worst := b.worst()
		if worst.Score <= tolerance+1e-6 {
			return math.Max(worst.Score, selectedError), nil
		}
		if len(b.Points) >= 65535 {
			return 0, fmt.Errorf("誤差補修でuint16の上限に到達")
		}
		if stop != nil && stop() {
			return 0, fmt.Errorf("階層誤差補修を保存前に停止")
		}
		p := point{int64(worst.Sample % b.N), int64(worst.Sample / b.N)}
		if _, ok := g.Selected[p]; ok {
			return 0, fmt.Errorf("階層誤差候補が既存点")
		}
		if _, e := g.insert(p); e != nil {
			return 0, e
		}
		if len(b.Heap) > 4*len(b.Tris)+64 {
			b.Heap = nil
			for _, id := range b.ids() {
				heap.Push(&b.Heap, b.Tris[id].E)
			}
		}
	}
}
