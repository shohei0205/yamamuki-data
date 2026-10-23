package main

import "fmt"

type tfcState struct {
	A, B                  [15]uint16
	V                     [12]uint16
	Edges, Vertices, Last int
}

func (s *tfcState) update(a, b, c int) {
	copy(s.A[3:], s.A[:12])
	copy(s.B[3:], s.B[:12])
	s.A[0], s.B[0] = uint16(b), uint16(a)
	s.A[1], s.B[1] = uint16(c), uint16(b)
	s.A[2], s.B[2] = uint16(a), uint16(c)
	s.Edges = min(15, s.Edges+3)
	for _, v := range []int{a, b, c} {
		at := 0
		for at < s.Vertices && int(s.V[at]) != v {
			at++
		}
		if at == s.Vertices {
			s.Vertices = min(12, s.Vertices+1)
		}
		k := min(at, 11)
		copy(s.V[1:k+1], s.V[:k])
		s.V[0] = uint16(v)
	}
	s.Last = c
}
func tfcZig(v int) int {
	if v >= 0 {
		return v * 2
	}
	return -v*2 - 1
}
func tfcSigned(v int) int {
	if v&1 != 0 {
		return -(v + 1) / 2
	}
	return v / 2
}
func (s *tfcState) token(a, b, c int) (int, int) {
	for i := 0; i < s.Vertices; i++ {
		if int(s.V[i]) == c {
			return i, 0
		}
	}
	if c == a+1 {
		return 12, 0
	}
	if c == b+1 {
		return 13, 0
	}
	if c == s.Last+1 {
		return 14, 0
	}
	fromB := absInt(c-b) < absInt(c-s.Last)
	d := c - s.Last
	bit := 0
	if fromB {
		d = c - b
		bit = 1
	}
	return 15, tfcZig(d)*2 + bit
}
func absInt(v int) int {
	if v < 0 {
		return -v
	}
	return v
}
func tfcLength(v int) int {
	if v < 128 {
		return 1
	}
	if v < 16384 {
		return 2
	}
	return 3
}
func encodeTFC(faces []face) ([]byte, error) {
	s := tfcState{}
	out := []byte{0xc1}
	visited := make([]bool, len(faces))
	type neighbor struct{ F, C int }
	adj := map[edge]neighbor{}
	for i, f := range faces {
		for j := 0; j < 3; j++ {
			a, b, c := f[j], f[(j+1)%3], f[(j+2)%3]
			if a < 0 || a > 65535 || b < 0 || b > 65535 || c < 0 || c > 65535 {
				return nil, fmt.Errorf("TFC1の番号上限")
			}
			ed := edge{a, b}
			if _, ok := adj[ed]; ok {
				return nil, fmt.Errorf("TFC1の有向辺が重複")
			}
			adj[ed] = neighbor{i, c}
		}
	}
	write := func(v int) {
		for v >= 128 {
			out = append(out, byte(v&127)|128)
			v >>= 7
		}
		out = append(out, byte(v))
	}
	first := 0
	for step := 0; step < len(faces); step++ {
		chosen, best, a, b, c, ed, tag, payload := -1, int(^uint(0)>>1), 0, 0, 0, -1, 0, 0
		for e := 0; e < s.Edges; e++ {
			n, ok := adj[edge{int(s.A[e]), int(s.B[e])}]
			if !ok || visited[n.F] {
				continue
			}
			t, p := s.token(int(s.A[e]), int(s.B[e]), n.C)
			live := 0
			for _, v := range edges(faces[n.F]) {
				if other, ok := adj[edge{v[1], v[0]}]; ok && !visited[other.F] {
					live++
				}
			}
			cost := 1
			if t == 15 {
				cost += tfcLength(p)
			}
			score := cost*100000 + live*100 + e
			if score < best {
				best = score
				chosen = n.F
				a = int(s.A[e])
				b = int(s.B[e])
				c = n.C
				ed = e
				tag = t
				payload = p
			}
		}
		if chosen < 0 {
			for first < len(faces) && visited[first] {
				first++
			}
			chosen = first
			a, b, c = faces[chosen][0], faces[chosen][1], faces[chosen][2]
			for e := 0; e < s.Edges; e++ {
				x, y := int(s.A[e]), int(s.B[e])
				v := -1
				if x == a && y == b {
					v = c
				} else if x == b && y == c {
					v = a
				} else if x == c && y == a {
					v = b
				}
				if v < 0 {
					continue
				}
				t, p := s.token(x, y, v)
				cost := 1
				if t == 15 {
					cost += tfcLength(p)
				}
				if cost < best {
					best = cost
					ed = e
					tag = t
					payload = p
				}
			}
			if ed >= 0 {
				x, y := int(s.A[ed]), int(s.B[ed])
				v := b
				if x == a && y == b {
					v = c
				} else if x == b && y == c {
					v = a
				}
				a, b, c = x, y, v
			}
		}
		visited[chosen] = true
		if ed >= 0 {
			out = append(out, byte(ed*16+tag))
			if tag == 15 {
				write(payload)
			}
		} else {
			out = append(out, 240)
			previous := s.Last
			for _, v := range []int{a, b, c} {
				write(tfcZig(v - previous))
				previous = v
			}
		}
		s.update(a, b, c)
	}
	return out, nil
}
func decodeTFC(src []byte, count, nv int) ([]face, error) {
	if count < 0 || count > 2000000 || nv < 1 || nv > 65536 || len(src) == 0 || src[0] != 0xc1 {
		return nil, fmt.Errorf("TFC1の形式・個数が不正")
	}
	s := tfcState{}
	cursor := 1
	read := func() (int, error) {
		v := 0
		for i := 0; i < 3; i++ {
			if cursor >= len(src) {
				return 0, fmt.Errorf("TFC1の途中切断")
			}
			b := src[cursor]
			cursor++
			v |= int(b&127) << (7 * i)
			if b < 128 {
				return v, nil
			}
		}
		return 0, fmt.Errorf("TFC1の整数超過")
	}
	out := make([]face, 0, count)
	for i := 0; i < count; i++ {
		if cursor >= len(src) {
			return nil, fmt.Errorf("TFC1の面が不足")
		}
		code := int(src[cursor])
		cursor++
		a, b, c := 0, 0, 0
		if code == 240 {
			previous := s.Last
			values := [3]int{}
			for j := 0; j < 3; j++ {
				v, e := read()
				if e != nil {
					return nil, e
				}
				values[j] = previous + tfcSigned(v)
				previous = values[j]
			}
			a, b, c = values[0], values[1], values[2]
		} else {
			e, t := code>>4, code&15
			if code > 240 || e >= s.Edges {
				return nil, fmt.Errorf("TFC1の辺参照不正")
			}
			a, b = int(s.A[e]), int(s.B[e])
			if t < 12 {
				if t >= s.Vertices {
					return nil, fmt.Errorf("TFC1の頂点参照不正")
				}
				c = int(s.V[t])
			} else if t == 12 {
				c = a + 1
			} else if t == 13 {
				c = b + 1
			} else if t == 14 {
				c = s.Last + 1
			} else {
				p, e := read()
				if e != nil {
					return nil, e
				}
				base := s.Last
				if p&1 != 0 {
					base = b
				}
				c = base + tfcSigned(p>>1)
			}
		}
		if a < 0 || b < 0 || c < 0 || a >= nv || b >= nv || c >= nv {
			return nil, fmt.Errorf("TFC1の頂点番号超過")
		}
		out = append(out, face{a, b, c})
		s.update(a, b, c)
	}
	if cursor != len(src) {
		return nil, fmt.Errorf("TFC1に余分なデータ")
	}
	return out, nil
}
