package main

import (
	"bytes"
	"compress/gzip"
	"database/sql"
	"fmt"
	"github.com/ulikunitz/xz"
	"io"
	"math"
)

type source struct {
	DB          *sql.DB
	Tiles       map[key][]float32
	Order       []key
	NN, FF, Rho float64
}

func newSource(path string) (*source, error) {
	db, e := openDB(path, true)
	if e != nil {
		return nil, e
	}
	p1, p2 := 30*math.Pi/180, 46*math.Pi/180
	nn := math.Log(math.Cos(p1)/math.Cos(p2)) / math.Log(math.Tan(math.Pi/4+p2/2)/math.Tan(math.Pi/4+p1/2))
	ff := math.Cos(p1) * math.Pow(math.Tan(math.Pi/4+p1/2), nn) / nn
	return &source{db, map[key][]float32{}, nil, nn, ff, 6378137 * ff / math.Pow(math.Tan(math.Pi/4+36*math.Pi/180/2), nn)}, nil
}
func decodeGrid(b []byte) ([]float32, error) {
	if len(b) < 18 || string(b[:4]) != "YMD1" {
		return nil, fmt.Errorf("元標高の形式が不正です")
	}
	rows, cols := int(le.Uint16(b[4:])), int(le.Uint16(b[6:]))
	step := math.Float32frombits(le.Uint32(b[8:]))
	base := int64(int32(le.Uint32(b[12:])))
	flags, codec := b[16], b[17]
	if rows != 256 || cols != 256 || step <= 0 || step > 5 || flags > 3 || codec > 1 {
		return nil, fmt.Errorf("元標高の保存条件が不正です")
	}
	width := 2
	if flags&1 != 0 {
		width = 4
	}
	n := rows * cols
	size := n * width
	if flags&2 != 0 {
		size += (n + 7) / 8
	}
	var rd io.Reader
	var e error
	if codec == 0 {
		r, err := gzip.NewReader(bytes.NewReader(b[18:]))
		if err != nil {
			return nil, err
		}
		defer r.Close()
		rd = r
	} else {
		rd, e = xz.NewReader(bytes.NewReader(b[18:]))
		if e != nil {
			return nil, e
		}
	}
	raw, e := io.ReadAll(io.LimitReader(rd, int64(size+1)))
	if e != nil {
		return nil, e
	}
	if len(raw) != size {
		return nil, fmt.Errorf("元標高の長さが不正です")
	}
	sums := make([]int64, n)
	v := make([]float32, n)
	for y := 0; y < rows; y++ {
		for x := 0; x < cols; x++ {
			i := y*cols + x
			delta := int64(int16(le.Uint16(raw[i*width:])))
			if width == 4 {
				delta = int64(int32(le.Uint32(raw[i*4:])))
			}
			if y > 0 {
				delta += sums[i-cols]
			}
			if x > 0 {
				delta += sums[i-1]
			}
			if y > 0 && x > 0 {
				delta -= sums[i-cols-1]
			}
			sums[i] = delta
			v[i] = float32(float64(delta+base) * float64(step))
			if flags&2 != 0 && raw[n*width+i/8]&(1<<uint(i%8)) != 0 {
				v[i] = float32(math.NaN())
			}
		}
	}
	return v, nil
}
func floorDiv(a, b int64) int64 {
	q := a / b
	if a%b < 0 {
		q--
	}
	return q
}
func (s *source) pixel(x, y int64) (float64, bool, error) {
	k := key{floorDiv(x, 256), floorDiv(y, 256)}
	grid, ok := s.Tiles[k]
	if !ok {
		var body []byte
		e := s.DB.QueryRow("SELECT data FROM tiles WHERE z=14 AND x=? AND y=?", k.X, k.Y).Scan(&body)
		if e != nil && e != sql.ErrNoRows {
			return 0, false, e
		}
		if e == nil {
			grid, e = decodeGrid(body)
			if e != nil {
				return 0, false, e
			}
		}
		if len(s.Tiles) >= 1024 {
			delete(s.Tiles, s.Order[0])
			s.Order = s.Order[1:]
		}
		s.Tiles[k] = grid
		s.Order = append(s.Order, k)
	}
	if grid == nil {
		return 0, false, nil
	}
	v := float64(grid[(y-k.Y*256)*256+x-k.X*256])
	return v, finite(v), nil
}
func (s *source) at(x, y float64) (float64, bool, error) {
	rho := math.Hypot(x, s.Rho-y)
	theta := math.Atan2(x, s.Rho-y)
	lon := (theta/s.NN + 138*math.Pi/180) * 180 / math.Pi
	lat := 2*math.Atan(math.Pow(6378137*s.FF/rho, 1/s.NN)) - math.Pi/2
	px := (lon+180)/360*4194304 - .5
	py := (1-math.Asinh(math.Tan(lat))/math.Pi)/2*4194304 - .5
	ix, iy := int64(math.Floor(px)), int64(math.Floor(py))
	fx, fy := px-float64(ix), py-float64(iy)
	value, weight := 0., 0.
	weights := [4]float64{(1 - fx) * (1 - fy), fx * (1 - fy), (1 - fx) * fy, fx * fy}
	for i, w := range weights {
		v, valid, e := s.pixel(ix+int64(i%2), iy+int64(i/2))
		if e != nil {
			return 0, false, e
		}
		if valid {
			value += v * w
			weight += w
		}
	}
	if weight <= .999999 {
		return 0, false, nil
	}
	return math.RoundToEven(float64(float32(value)*4)) / 4, true, nil
}
func gain(s *source, j job) ([]float64, []float64, error) {
	n := j.n()
	w := n + 64
	z := make([]float64, w*w)
	valid := make([]bool, w*w)
	grid := make([]float64, n*n)
	for y := 0; y < w; y++ {
		for x := 0; x < w; x++ {
			v, ok, e := s.at(float64(j.K.X)*j.width()+float64(x-32)*31.25, float64(j.K.Y)*j.width()+float64(y-32)*31.25)
			if e != nil {
				return nil, nil, e
			}
			z[y*w+x] = v
			valid[y*w+x] = ok
			if x >= 32 && y >= 32 && x < 32+n && y < 32+n {
				grid[(y-32)*n+x-32] = v
			}
		}
	}
	kernel := make([]float64, 33)
	sum := 0.
	for i := range kernel {
		k := i - 16
		kernel[i] = math.Exp(-float64(k*k) / 32)
		sum += kernel[i]
	}
	temp := make([]float64, w*w)
	for y := 0; y < w; y++ {
		for x := 0; x < w; x++ {
			v := 0.
			for k, a := range kernel {
				v += z[y*w+max(0, min(w-1, x+k-16))] * a
			}
			temp[y*w+x] = v / sum
		}
	}
	// 元の標高はgridへ移したので、zを滑らかな標高として再利用する。
	for y := 0; y < w; y++ {
		for x := 0; x < w; x++ {
			v := 0.
			for k, a := range kernel {
				v += temp[max(0, min(w-1, y+k-16))*w+x] * a
			}
			z[y*w+x] = v / sum
		}
	}
	// 水平フィルターの一時領域を勾配に再利用する。
	clear(temp)
	for y := 1; y < w-1; y++ {
		for x := 1; x < w-1; x++ {
			temp[y*w+x] = math.Hypot((z[y*w+x+1]-z[y*w+x-1])/62.5, (z[(y+1)*w+x]-z[(y-1)*w+x])/62.5)
		}
	}
	spare := make([]float64, w*w)
	for _, vertical := range []bool{false, true} {
		for line := 0; line < w; line++ {
			index := func(a int) int {
				if vertical {
					return a*w + line
				}
				return line*w + a
			}
			queue := make([]int, 0, 32)
			head, next := 0, 0
			for at := 0; at < w; at++ {
				hi := min(at+8, w-1)
				for next <= hi {
					for len(queue) > head && temp[index(queue[len(queue)-1])] <= temp[index(next)] {
						queue = queue[:len(queue)-1]
					}
					queue = append(queue, next)
					next++
				}
				for queue[head]+8 < at {
					head++
				}
				spare[index(at)] = temp[index(queue[head])]
			}
		}
		temp, spare = spare, temp
	}
	// 使い終わったバッファを許容誤差格子へ転用する。
	limits := spare[:n*n]
	low, high := math.Tan(math.Pi/180), math.Tan(8*math.Pi/180)
	for y := 32; y < n+32; y++ {
		for x := 32; x < n+32; x++ {
			i := y*w + x
			t := max(0, min(1, (temp[i]-low)/(high-low)))
			convex := max(0, min(1, (4*z[i]-z[i-8]-z[i+8]-z[i-8*w]-z[i+8*w])/50))
			weight := .02
			if valid[i] {
				weight = (.02 + .98*t*t*(3-2*t)) * (1 + .5*convex)
			}
			limits[(y-32)*n+x-32] = j.tolerance() / weight
		}
	}
	return grid, limits, nil
}
