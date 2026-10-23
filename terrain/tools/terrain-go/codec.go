package main

import (
	"bytes"
	"compress/zlib"
	"database/sql"
	"encoding/binary"
	"errors"
	"fmt"
	"github.com/klauspost/compress/zstd"
	"io"
	"math"
	_ "modernc.org/sqlite"
	"net/url"
	"path/filepath"
	"strings"
)

var le = binary.LittleEndian
var zdecoder, _ = zstd.NewReader(nil, zstd.WithDecoderConcurrency(1))

func unzip(b []byte) ([]byte, error) {
	if len(b) >= 4 && string(b[:4]) == "YQW1" {
		return zdecoder.DecodeAll(b[4:], nil)
	}
	r, e := zlib.NewReader(bytes.NewReader(b))
	if e != nil {
		return nil, e
	}
	defer r.Close()
	return io.ReadAll(io.LimitReader(r, 1024*1024*1024))
}
func zipBytes(b []byte, level int) ([]byte, error) {
	var out bytes.Buffer
	w, e := zlib.NewWriterLevel(&out, level)
	if e != nil {
		return nil, e
	}
	if _, e = w.Write(b); e != nil {
		return nil, e
	}
	if e = w.Close(); e != nil {
		return nil, e
	}
	return out.Bytes(), nil
}
func decodeMesh(raw []byte, n int) ([]point, []face, error) {
	if len(raw) < 12 || string(raw[:4]) != "YQB1" {
		return nil, nil, fmt.Errorf("YQB1形式ではありません")
	}
	nv, nf := int(le.Uint32(raw[4:8])), int(le.Uint32(raw[8:12]))
	if nv < 4 || len(raw) != 12+12*(nv+nf) {
		return nil, nil, fmt.Errorf("メッシュサイズが不正です")
	}
	p := make([]point, nv)
	f := make([]face, nf)
	for i := range p {
		xy := [2]int64{}
		for k := 0; k < 2; k++ {
			v := float64(math.Float32frombits(le.Uint32(raw[12+i*12+k*4:]))) / 31.25
			if !finite(v) || v != math.Round(v) || v < 0 || v >= float64(n) {
				return nil, nil, fmt.Errorf("格子外の頂点")
			}
			xy[k] = int64(v)
		}
		p[i] = point{xy[0], xy[1]}
	}
	for i := range f {
		for k := 0; k < 3; k++ {
			id := int(le.Uint32(raw[12+nv*12+i*12+k*4:]))
			if id >= nv {
				return nil, nil, fmt.Errorf("頂点番号が範囲外です")
			}
			f[i][k] = id
		}
	}
	return p, f, nil
}
func (b *builder) raw() []byte {
	out := bytes.NewBuffer(make([]byte, 0, 12+len(b.Points)*12+len(b.Tris)*12))
	out.WriteString("YQB1")
	binary.Write(out, le, uint32(len(b.Points)))
	binary.Write(out, le, uint32(len(b.Tris)))
	for _, p := range b.Points {
		for _, v := range []float64{float64(p.X) * 31.25, float64(p.Y) * 31.25, b.Grid[int(p.Y)*b.N+int(p.X)]} {
			binary.Write(out, le, float32(v))
		}
	}
	for _, id := range b.ids() {
		for _, i := range b.Tris[id].F {
			binary.Write(out, le, uint32(i))
		}
	}
	return out.Bytes()
}
func decodeFields(raw []byte, n int) ([]float64, []float64, error) {
	count := n * n
	if len(raw) != count*16 {
		return nil, nil, fmt.Errorf("標高キャッシュのサイズが不正です")
	}
	a := make([]float64, count)
	b := make([]float64, count)
	for i := range a {
		a[i] = math.Float64frombits(le.Uint64(raw[i*8:]))
		b[i] = math.Float64frombits(le.Uint64(raw[(count+i)*8:]))
	}
	return a, b, nil
}
func openDB(path string, ro bool) (*sql.DB, error) {
	abs, e := filepath.Abs(path)
	if e != nil {
		return nil, e
	}
	abs = strings.TrimPrefix(abs, `\\?\`)
	uriPath := filepath.ToSlash(abs)
	if filepath.VolumeName(abs) != "" {
		uriPath = "/" + uriPath
	}
	u := url.URL{Scheme: "file", Path: uriPath}
	q := u.Query()
	if ro {
		q.Set("mode", "ro")
	}
	q.Add("_pragma", "busy_timeout(30000)")
	u.RawQuery = q.Encode()
	d, e := sql.Open("sqlite", u.String())
	if e != nil {
		return nil, e
	}
	d.SetMaxOpenConns(16)
	return d, d.Ping()
}

type key struct{ X, Y int64 }
type job struct {
	Q int
	K key
}

func (j job) width() float64     { return 2000 * math.Exp2(float64(j.Q-4)) }
func (j job) n() int             { return int(j.width()/31.25) + 1 }
func (j job) tolerance() float64 { return 12.5 * math.Exp2(float64(j.Q-4)) }

type queryer interface{ QueryRow(string, ...any) *sql.Row }

var errNoFields = errors.New("標高キャッシュなし")

func fieldData(db queryer, cache *sql.DB, j job) ([]float64, []float64, error) {
	for _, t := range []string{"fields_fast", "fields"} {
		readers := []queryer{db}
		if cache != nil {
			readers = append(readers, cache)
		}
		for _, d := range readers {
			var b []byte
			e := d.QueryRow("SELECT data FROM "+t+" WHERE q=? AND x=? AND y=?", j.Q, j.K.X, j.K.Y).Scan(&b)
			if e == sql.ErrNoRows {
				continue
			}
			if e != nil {
				if strings.Contains(e.Error(), "no such table") {
					continue
				}
				return nil, nil, e
			}
			var raw []byte
			if t == "fields_fast" {
				raw, e = zdecoder.DecodeAll(b, nil)
			} else {
				raw, e = unzip(b)
			}
			if e != nil {
				if strings.Contains(e.Error(), "no such table") {
					continue
				}
				return nil, nil, e
			}
			return decodeFields(raw, j.n())
		}
	}
	return nil, nil, fmt.Errorf("%w: Q%d %v", errNoFields, j.Q, j.K)
}
