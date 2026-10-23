package main

import (
	"container/heap"
	"math"
)

type density struct {
	Counts       map[key]uint64
	Stride, Step int64
}

func newDensity(j job) *density { s := int64(j.n() - 1); return &density{map[key]uint64{}, s, s / 8} }
func (d *density) add(p point) {
	x, y, r := float64(p.X)/float64(d.Step), float64(p.Y)/float64(d.Step), float64(d.Stride)/(2*float64(d.Step))
	for yy := int64(math.Ceil(y - r)); yy <= int64(math.Floor(y+r)); yy++ {
		for xx := int64(math.Ceil(x - r)); xx <= int64(math.Floor(x+r)); xx++ {
			if (float64(xx)-x)*(float64(xx)-x)+(float64(yy)-y)*(float64(yy)-y) <= r*r+1e-9 {
				d.Counts[key{xx, yy}]++
			}
		}
	}
}
func (d *density) tile(k key) []float64 {
	a := make([]float64, 81)
	for y := int64(0); y <= 8; y++ {
		for x := int64(0); x <= 8; x++ {
			a[y*9+x] = float64(d.Counts[key{k.X*8 + x, k.Y*8 + y}])
		}
	}
	return a
}
func (d *density) world(k key, sample int) point {
	return point{k.X*d.Stride + int64(sample%int(d.Stride+1)), k.Y*d.Stride + int64(sample/int(d.Stride+1))}
}

type candidate struct {
	Score float64
	K     key
}
type candidateHeap []candidate

func (h candidateHeap) Len() int { return len(h) }
func (h candidateHeap) Less(i, j int) bool {
	a, b := h[i], h[j]
	if a.Score != b.Score {
		return a.Score > b.Score
	}
	if a.K.X != b.K.X {
		return a.K.X < b.K.X
	}
	return a.K.Y < b.K.Y
}
func (h candidateHeap) Swap(i, j int) { h[i], h[j] = h[j], h[i] }
func (h *candidateHeap) Push(v any)   { *h = append(*h, v.(candidate)) }
func (h *candidateHeap) Pop() any     { a := *h; v := a[len(a)-1]; *h = a[:len(a)-1]; return v }
func (h *candidateHeap) add(k key, s float64) {
	if s > .1 {
		heap.Push(h, candidate{s, k})
	}
}
// 点ごとのハッシュ更新をタイル内配列へまとめ、同じ密度を少ない書込で集計する。
func(d *density)addBasicTile(k key,points []point,shared map[point]bool)uint64{
    var local [17*17]uint64;var count uint64
    for _,p:=range points {world:=point{k.X*d.Stride+p.X,k.Y*d.Stride+p.Y};onEdge:=p.X==0||p.Y==0||p.X==d.Stride||p.Y==d.Stride;if onEdge {if shared[world]{continue};shared[world]=true};count++
        x,y:=float64(p.X)/float64(d.Step),float64(p.Y)/float64(d.Step)
        for yy:=int(math.Ceil(y-4));yy<=int(math.Floor(y+4));yy++{for xx:=int(math.Ceil(x-4));xx<=int(math.Floor(x+4));xx++{dx,dy:=float64(xx)-x,float64(yy)-y;if dx*dx+dy*dy<=16+1e-9{local[(yy+4)*17+xx+4]++}}}
    }
    for i,v:=range local{if v>0{d.Counts[key{k.X*8+int64(i%17)-4,k.Y*8+int64(i/17)-4}]+=v}};return count
}
