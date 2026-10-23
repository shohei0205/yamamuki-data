package main

import (
	"bytes"
	"crypto/sha256"
	"database/sql"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/klauspost/compress/zstd"
	"math"
	"os"
	"path/filepath"
	"runtime/debug"
	"sort"
	"strings"
	"sync"
	"time"
)

type hierarchyPipeline struct {
	R                   *runner
	Original, HB, Final *sql.DB
	Manifest            string
}
type hierarchyBuilt struct {
	K              key
	Raw, Validity  []byte
	Maximum, Angle float64
	Added          int
	Err            error
}

const hierarchySchema = `CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);
 CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS basic(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,source_validity BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS basic_stats(q INTEGER,x INTEGER,y INTEGER,engine TEXT,max_error REAL,min_angle REAL,rounds INTEGER,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS border_points(q INTEGER,x INTEGER,y INTEGER,z REAL NOT NULL,PRIMARY KEY(q,x,y));
 CREATE INDEX IF NOT EXISTS border_y ON border_points(q,y,x);`
const hierarchyPoolSchema = `CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);
 CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS additional(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,added_vertex_ids BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS working(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,ids BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS candidates(q INTEGER,x INTEGER,y INTEGER,score REAL,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS trace(q INTEGER,seq INTEGER,x INTEGER,y INTEGER,sample INTEGER,priority REAL,PRIMARY KEY(q,seq));
 CREATE TABLE IF NOT EXISTS fields(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS fields_fast(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));`

func checkedHierarchyDB(path, config, schema string) (*sql.DB, error) {
	db, e := openDB(path, false)
	if e != nil {
		return nil, e
	}
	var count int
	if e = db.QueryRow("SELECT count(*) FROM sqlite_master WHERE type='table'").Scan(&count); e != nil {
		db.Close()
		return nil, e
	}
	if count > 0 {
		var old string
		if e = db.QueryRow("SELECT value FROM metadata WHERE key='hierarchy_config'").Scan(&old); e != nil || old != config {
			db.Close()
			return nil, fmt.Errorf("階層出力の条件が異なります: %s", path)
		}
	}
	if _, e = db.Exec("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;" + schema); e != nil {
		db.Close()
		return nil, e
	}
	_, e = db.Exec("INSERT OR IGNORE INTO metadata VALUES('hierarchy_config',?),('complete','false'),('distributable','false')", config)
	if e != nil {
		db.Close()
		return nil, e
	}
	return db, nil
}
func executeHierarchy(o options) error {
	if o.Base == "" || o.Output == "" || o.HierarchyOutput == "" || o.FinalOutput == "" || o.Workers < 1 || o.MinQ < 4 || o.MaxQ > 12 || o.MinQ > o.MaxQ || o.Limit >= 0 {
		return fmt.Errorf("階層生成の引数が不正です。試験は入力目録を小さくし、点数制限は使わないでください")
	}
	paths := []string{o.Base, o.Output, o.HierarchyOutput, o.FinalOutput}
	seen := map[string]bool{}
	for i, p := range paths {
		a, e := filepath.Abs(p)
		if e != nil {
			return e
		}
		paths[i] = a
		normalized := strings.ToLower(a)
		if seen[normalized] {
			return fmt.Errorf("階層出力と入力は別ファイルにしてください")
		}
		seen[normalized] = true
		if i > 0 {
			if e = os.MkdirAll(filepath.Dir(a), 0700); e != nil {
				return e
			}
		}
	}
	o.Base, o.Output, o.HierarchyOutput, o.FinalOutput = paths[0], paths[1], paths[2], paths[3]
	if o.Cache != "" {
		a, _ := filepath.Abs(o.Cache)
		for _, p := range paths[1:] {
			if strings.EqualFold(a, p) {
				return fmt.Errorf("キャッシュへ出力できません")
			}
		}
	}
	unlocks := []func(){}
	defer func() {
		for _, u := range unlocks {
			u()
		}
	}()
	for _, p := range paths[1:] {
		u, e := lockOutput(stem(p) + ".go-lock")
		if e != nil {
			return e
		}
		unlocks = append(unlocks, u)
	}
	original, e := openDB(o.Base, true)
	if e != nil {
		return e
	}
	defer original.Close()
	var pending int
	if e = original.QueryRow("SELECT count(*) FROM jobs WHERE state NOT IN(1,2)").Scan(&pending); e != nil || pending != 0 {
		return fmt.Errorf("元の基本メッシュが未完了: %v", e)
	}
	var manifest string
	if e = original.QueryRow("SELECT value FROM metadata WHERE key='manifest'").Scan(&manifest); e != nil {
		return e
	}
	var m map[string]any
	if e = json.Unmarshal([]byte(manifest), &m); e != nil {
		return e
	}
	if p, ok := m["source"].(string); ok {
		o.Source = p
	}
	var info struct {
		SourceSize    int64
		SourceMtimeNs int64
	}
	if e = json.Unmarshal([]byte(manifest), &info); e != nil {
		return e
	}
	stat, e := os.Stat(o.Source)
	if e != nil {
		return e
	}
	if stat.Size() != info.SourceSize || stat.ModTime().UnixNano() != info.SourceMtimeNs {
		return fmt.Errorf("元標高が基本生成時と不一致")
	}
	configBody, _ := json.Marshal(map[string]any{"version": 1, "policy": "hierarchical-final-parent-v1", "sourceBase": o.Base, "manifest": json.RawMessage(manifest), "minimumQ": o.MinQ, "maximumQ": o.MaxQ, "densityFreeVertices": o.Free, "additionalFraction": .2, "minimumAngle": 10, "codec": "TFC1", "triangleOrder": "tfc1-shared-edge-greedy", "outerCompression": "none"})
	config := string(configBody)
	hb, e := checkedHierarchyDB(o.HierarchyOutput, config, hierarchySchema)
	if e != nil {
		return e
	}
	defer hb.Close()
	pool, e := checkedHierarchyDB(o.Output, config, hierarchyPoolSchema)
	if e != nil {
		return e
	}
	defer pool.Close()
	final, e := checkedHierarchyDB(o.FinalOutput, config, `CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(q INTEGER,x INTEGER,y INTEGER,parent_q INTEGER,parent_x INTEGER,parent_y INTEGER,vertices INTEGER,triangles INTEGER,inherited INTEGER,parent_refs BLOB,new_vertices BLOB,indices BLOB,border_indices BLOB,mesh_sha256 TEXT,PRIMARY KEY(q,x,y));`)
	if e != nil {
		return e
	}
	defer final.Close()
	hb.Exec("INSERT OR IGNORE INTO metadata VALUES('manifest',?)", manifest)
	final.Exec("INSERT OR IGNORE INTO metadata VALUES('xyUnitMeters','31.25'),('heightUnitMeters','0.25'),('heightEncoding','signed-int16'),('vertexEncoding','inherited-prefix-plus-uint16-xy-int16-z'),('parentRefs','uint16-parent-index'),('triangleEncoding','TFC1'),('boundaryConnectionComplete','false')")
	r := &runner{O: o, Base: hb, DB: pool, Start: time.Now()}
	h := &hierarchyPipeline{r, original, hb, final, manifest}
	if o.Cache != "" {
		r.Cache, e = openDB(o.Cache, true)
		if e != nil {
			return e
		}
		defer r.Cache.Close()
		var c string
		if e = r.Cache.QueryRow("SELECT value FROM metadata WHERE key='additional_config'").Scan(&c); e != nil {
			return e
		}
		var v struct {
			Base struct{ Manifest json.RawMessage }
		}
		json.Unmarshal([]byte(c), &v)
		var a, b any
		json.Unmarshal(v.Base.Manifest, &a)
		json.Unmarshal([]byte(manifest), &b)
		if !sameFieldSource(a, b) {
			return fmt.Errorf("階層キャッシュの元標高条件不一致")
		}
	}
	debug.SetMemoryLimit(int64(min(float64(availableMemory())*.65, (o.CacheGiB+2)*1073741824)))
	if e = h.initializeJobs(); e != nil {
		return e
	}
	for q := o.MaxQ; q >= o.MinQ; q-- {
		var packed int
		final.QueryRow("SELECT count(*) FROM metadata WHERE key=?", fmt.Sprintf("q%d-complete", q)).Scan(&packed)
		if packed > 0 {
			continue
		}
		if r.stop() {
			r.status("paused", q, 0, 0, 0, 0)
			return nil
		}
		var done int
		pool.QueryRow("SELECT count(*) FROM metadata WHERE key=?", fmt.Sprintf("q%d-complete", q)).Scan(&done)
		if done == 0 {
			if e = h.seedBorders(q); e != nil {
				return e
			}
			if e = h.buildLevel(q); e != nil {
				if r.stop() {
					r.status("paused", q, 0, 0, 0, 0)
					return nil
				}
				return e
			}
			if stopped, e := r.level(q); e != nil {
				return e
			} else if stopped {
				return nil
			}
		}
		if e = h.packLevel(q); e != nil {
			return e
		}
		h.basicStatus("hierarchical-level-ready", q)
	}
	final.Exec("INSERT OR REPLACE INTO metadata VALUES('stage','hierarchical-ready-for-boundary-validation')")
	return r.status("hierarchical-ready-for-boundary-validation", o.MinQ, 0, 0, 0, 0)
}
func (h *hierarchyPipeline) initializeJobs() error {
	var count int
	h.HB.QueryRow("SELECT count(*) FROM jobs").Scan(&count)
	if count > 0 {
		return nil
	}
	rows, e := h.Original.Query("SELECT q,x,y,state FROM jobs WHERE q BETWEEN ? AND ? ORDER BY q,x,y", h.R.O.MinQ, h.R.O.MaxQ)
	if e != nil {
		return e
	}
	defer rows.Close()
	tx, e := h.HB.Begin()
	if e != nil {
		return e
	}
	defer tx.Rollback()
	for rows.Next() {
		var q, state int
		var x, y int64
		if e = rows.Scan(&q, &x, &y, &state); e != nil {
			return e
		}
		if state == 1 {
			state = 0
		}
		if _, e = tx.Exec("INSERT INTO jobs(q,x,y,state) VALUES(?,?,?,?)", q, x, y, state); e != nil {
			return e
		}
	}
	if e = rows.Err(); e != nil {
		return e
	}
	return tx.Commit()
}
func meshHeights(raw []byte, p []point) map[point]float64 {
	out := make(map[point]float64, len(p))
	for i, v := range p {
		out[v] = float64(math.Float32frombits(le.Uint32(raw[12+i*12+8:])))
	}
	return out
}
func (h *hierarchyPipeline) finished(q int, k key) ([]byte, error) {
	var body []byte
	e := h.R.DB.QueryRow("SELECT mesh FROM additional WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&body)
	if e == sql.ErrNoRows {
		e = h.HB.QueryRow("SELECT mesh FROM basic WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&body)
	}
	if e != nil {
		return nil, e
	}
	return unzip(body)
}
func addBorder(tx *sql.Tx, q int, p point, z float64) (bool, error) {
	result, e := tx.Exec("INSERT OR IGNORE INTO border_points VALUES(?,?,?,?)", q, p.X, p.Y, z)
	if e != nil {
		return false, e
	}
	n, _ := result.RowsAffected()
	if n > 0 {
		return true, nil
	}
	var old float64
	if e = tx.QueryRow("SELECT z FROM border_points WHERE q=? AND x=? AND y=?", q, p.X, p.Y).Scan(&old); e != nil {
		return false, e
	}
	if math.Abs(old-z) > .250001 {
		return false, fmt.Errorf("共有辺の元標高不一致: Q%d %v %.2f/%.2f", q, p, old, z)
	}
	return false, nil
}
func (h *hierarchyPipeline) seedBorders(q int) error {
	var done int
	name := fmt.Sprintf("q%d-borders-seeded", q)
	h.HB.QueryRow("SELECT count(*) FROM metadata WHERE key=?", name).Scan(&done)
	if done > 0 {
		return nil
	}
	h.R.status("hierarchical-seeding-borders", q, 0, 0, 0, 0)
	stride := int64(job{Q: q}.n() - 1)
	seed := func(raw []byte, k key, pn int, parent bool, tx *sql.Tx) error {
		p, _, e := decodeMesh(raw, pn)
		if e != nil {
			return e
		}
		z := meshHeights(raw, p)
		scale := stride
		if parent {
			scale *= 2
		}
		for _, v := range p {
			world := point{k.X*scale + v.X, k.Y*scale + v.Y}
			if world.X%stride == 0 || world.Y%stride == 0 {
				if _, e = addBorder(tx, q, world, z[v]); e != nil {
					return e
				}
			}
		}
		return nil
	}
	tx, e := h.HB.Begin()
	if e != nil {
		return e
	}
	defer tx.Rollback()
	if q < h.R.O.MaxQ {
		rows, e := h.HB.Query("SELECT x,y FROM basic WHERE q=? ORDER BY y,x", q+1)
		if e != nil {
			return e
		}
		for rows.Next() {
			var k key
			if e = rows.Scan(&k.X, &k.Y); e != nil {
				rows.Close()
				return e
			}
			raw, e := h.finished(q+1, k)
			if e != nil {
				rows.Close()
				return e
			}
			if e = seed(raw, k, job{Q: q + 1}.n(), true, tx); e != nil {
				rows.Close()
				return e
			}
		}
		if e = rows.Err(); e != nil {
			rows.Close()
			return e
		}
		rows.Close()
	}
	rows, e := h.Original.Query("SELECT x,y,mesh FROM basic WHERE q=? ORDER BY y,x", q)
	if e != nil {
		return e
	}
	defer rows.Close()
	for rows.Next() {
		if h.R.stop() {
			return fmt.Errorf("共有辺準備を停止")
		}
		var k key
		var body []byte
		if e = rows.Scan(&k.X, &k.Y, &body); e != nil {
			return e
		}
		raw, e := unzip(body)
		if e != nil {
			return e
		}
		if e = seed(raw, k, int(stride+1), false, tx); e != nil {
			return e
		}
	}
	if e = rows.Err(); e != nil {
		return e
	}
	if _, e = tx.Exec("INSERT INTO metadata VALUES(?,'true')", name); e != nil {
		return e
	}
	return tx.Commit()
}
func borderRows(db interface {
	Query(string, ...any) (*sql.Rows, error)
}, q int, k key) (map[point]float64, error) {
	stride := int64(job{Q: q}.n() - 1)
	left, top := k.X*stride, k.Y*stride
	rows, e := db.Query("SELECT x,y,z FROM border_points WHERE q=? AND ((x IN (?,?) AND y BETWEEN ? AND ?) OR (y IN (?,?) AND x BETWEEN ? AND ?))", q, left, left+stride, top, top+stride, top, top+stride, left, left+stride)
	if e != nil {
		return nil, e
	}
	defer rows.Close()
	out := map[point]float64{}
	for rows.Next() {
		var x, y int64
		var z float64
		if e = rows.Scan(&x, &y, &z); e != nil {
			return nil, e
		}
		out[point{x - left, y - top}] = z
	}
	return out, rows.Err()
}
func (h *hierarchyPipeline) fields(j job) ([]float64, error) {
	grid, limits, e := fieldData(h.R.DB, h.R.Cache, j)
	if e == nil {
		return grid, nil
	}
	if e != errNoFields {
		return nil, e
	}
	src, e := newSource(h.R.O.Source)
	if e != nil {
		return nil, e
	}
	grid, limits, e = gain(src, j)
	src.DB.Close()
	if e != nil {
		return nil, e
	}
	raw := make([]byte, len(grid)*16)
	for i, v := range grid {
		le.PutUint64(raw[i*8:], math.Float64bits(v))
	}
	for i, v := range limits {
		le.PutUint64(raw[(len(grid)+i)*8:], math.Float64bits(v))
	}
	encoder, e := zstd.NewWriter(nil, zstd.WithEncoderLevel(zstd.SpeedFastest), zstd.WithEncoderConcurrency(1))
	if e != nil {
		return nil, e
	}
	body := encoder.EncodeAll(raw, nil)
	encoder.Close()
	_, e = h.R.DB.Exec("INSERT OR IGNORE INTO fields_fast VALUES(?,?,?,?)", j.Q, j.K.X, j.K.Y, body)
	return grid, e
}
func (h *hierarchyPipeline) buildTile(q int, k key) hierarchyBuilt {
	out := hierarchyBuilt{K: k}
	j := job{q, k}
	var body []byte
	e := h.Original.QueryRow("SELECT mesh,source_validity FROM basic WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&body, &out.Validity)
	if e != nil {
		out.Err = e
		return out
	}
	raw, e := unzip(body)
	if e != nil {
		out.Err = e
		return out
	}
	p, f, e := decodeMesh(raw, j.n())
	if e != nil {
		out.Err = e
		return out
	}
	heights := meshHeights(raw, p)
	if e = h.Original.QueryRow("SELECT max_error,min_angle FROM basic_stats WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&out.Maximum, &out.Angle); e != nil {
		out.Err = e
		return out
	}
	required, e := borderRows(h.HB, q, k)
	if e != nil {
		out.Err = e
		return out
	}
	if q < h.R.O.MaxQ {
		pk := key{floorDiv(k.X, 2), floorDiv(k.Y, 2)}
		parent, e := h.finished(q+1, pk)
		if e != nil {
			out.Err = e
			return out
		}
		pp, _, e := decodeMesh(parent, job{Q: q + 1}.n())
		if e != nil {
			out.Err = e
			return out
		}
		zs := meshHeights(parent, pp)
		stride := int64(j.n() - 1)
		for _, v := range pp {
			local := point{v.X - (k.X-pk.X*2)*stride, v.Y - (k.Y-pk.Y*2)*stride}
			if local.X >= 0 && local.Y >= 0 && local.X <= stride && local.Y <= stride {
				if z, ok := required[local]; ok && math.Abs(z-zs[v]) > .250001 {
					out.Err = fmt.Errorf("親と共有辺の標高不一致")
					return out
				}
				required[local] = zs[v]
			}
		}
	}
	g, e := hierarchySeed(j.n(), p, f)
	if e != nil {
		out.Err = e
		return out
	}
	changed := false
	requiredPoints := make([]point, 0, len(required))
	for v := range required {
		requiredPoints = append(requiredPoints, v)
	}
	sort.Slice(requiredPoints, func(i, j int) bool {
		if requiredPoints[i].Y != requiredPoints[j].Y {
			return requiredPoints[i].Y < requiredPoints[j].Y
		}
		return requiredPoints[i].X < requiredPoints[j].X
	})
	for _, v := range requiredPoints {
		z := required[v]
		if old, ok := heights[v]; !ok {
			if _, e = g.insert(v); e != nil {
				out.Err = e
				return out
			}
			changed = true
		} else if old != z {
			changed = true
		}
		heights[v] = z
	}
	if !changed {
		out.Raw = raw
		return out
	}
	grid, e := h.fields(j)
	if e != nil {
		out.Err = e
		return out
	}
	maximum, e := refineHierarchy(g, grid, heights, j.tolerance(), h.R.stop)
	if e != nil {
		out.Err = e
		return out
	}
	out.Raw = g.B.raw()
	out.Maximum = maximum
	out.Angle = g.minimumAngle()
	out.Added = len(g.B.Points) - len(p)
	return out
}
func (h *hierarchyPipeline) basicStatus(stage string, q int) {
	var levels []any
	var m map[string]json.RawMessage
	json.Unmarshal([]byte(h.Manifest), &m)
	json.Unmarshal(m["levels"], &levels)
	rows, e := h.HB.Query("SELECT q,count(*),sum(vertices),sum(triangles),sum(length(mesh)),sum(length(source_validity)) FROM basic GROUP BY q ORDER BY q")
	sizes := [][6]int64{}
	if e == nil {
		for rows.Next() {
			var s [6]int64
			if rows.Scan(&s[0], &s[1], &s[2], &s[3], &s[4], &s[5]) == nil {
				sizes = append(sizes, s)
			}
		}
		rows.Close()
	}
	jsonFile(stem(h.R.O.HierarchyOutput)+".progress.json", map[string]any{"stage": stage, "q": q, "levels": levels, "sizes": sizes, "complete": false, "distributable": false, "updated": float64(time.Now().UnixNano()) / 1e9, "pid": os.Getpid()})
}
func (h *hierarchyPipeline) buildLevel(q int) error {
	for {
		if e := h.R.checkDisk(); e != nil {
			return e
		}
		if h.R.stop() {
			return fmt.Errorf("階層基本生成を停止")
		}
		n := uint64(job{Q: q}.n())
		workers := max(1, min(h.R.O.Workers, int(availableMemory()/(n*n*48+512*1024*1024))))
		rows, e := h.HB.Query("SELECT x,y FROM jobs WHERE q=? AND state=0 ORDER BY y,x LIMIT ?", q, workers)
		if e != nil {
			return e
		}
		keys := []key{}
		for rows.Next() {
			var k key
			if e = rows.Scan(&k.X, &k.Y); e != nil {
				rows.Close()
				return e
			}
			keys = append(keys, k)
		}
		if e = rows.Err(); e != nil {
			rows.Close()
			return e
		}
		rows.Close()
		if len(keys) == 0 {
			break
		}
		var completed, total int
		h.HB.QueryRow("SELECT count(*) FROM jobs WHERE q=? AND state=1", q).Scan(&completed)
		h.HB.QueryRow("SELECT count(*) FROM jobs WHERE q=? AND state!=2", q).Scan(&total)
		h.R.status("hierarchical-building-basic", q, completed, total, 0, 0)
		h.basicStatus("hierarchical-building-basic", q)
		results := make(chan hierarchyBuilt, len(keys))
		var wg sync.WaitGroup
		for _, k := range keys {
			wg.Add(1)
			go func(k key) { defer wg.Done(); results <- h.buildTile(q, k) }(k)
		}
		wg.Wait()
		close(results)
		batch := []hierarchyBuilt{}
		for v := range results {
			if v.Err != nil {
				return fmt.Errorf("Q%d %v の階層基本生成: %w", q, v.K, v.Err)
			}
			batch = append(batch, v)
		}
		sort.Slice(batch, func(i, j int) bool {
			if batch[i].K.Y != batch[j].K.Y {
				return batch[i].K.Y < batch[j].K.Y
			}
			return batch[i].K.X < batch[j].K.X
		})
		for _, v := range batch {
			if e = h.commitTile(q, v); e != nil {
				return e
			}
		}
		fmt.Printf("階層基本 Q%d %d/%d 並列%d\n", q, completed+len(keys), total, workers)
	}
	return nil
}
func (h *hierarchyPipeline) commitTile(q int, v hierarchyBuilt) error {
	j := job{q, v.K}
	p, f, e := decodeMesh(v.Raw, j.n())
	if e != nil {
		return e
	}
	zs := meshHeights(v.Raw, p)
	body, e := zipBytes(v.Raw, 6)
	if e != nil {
		return e
	}
	tx, e := h.HB.Begin()
	if e != nil {
		return e
	}
	defer tx.Rollback()
	stride := int64(j.n() - 1)
	for _, local := range p {
		if local.X != 0 && local.Y != 0 && local.X != stride && local.Y != stride {
			continue
		}
		world := point{v.K.X*stride + local.X, v.K.Y*stride + local.Y}
		added, e := addBorder(tx, q, world, zs[local])
		if e != nil {
			return e
		}
		if added {
			xs, ys := []int64{v.K.X}, []int64{v.K.Y}
			if local.X == 0 {
				xs = append(xs, v.K.X-1)
			}
			if local.X == stride {
				xs = append(xs, v.K.X+1)
			}
			if local.Y == 0 {
				ys = append(ys, v.K.Y-1)
			}
			if local.Y == stride {
				ys = append(ys, v.K.Y+1)
			}
			for _, x := range xs {
				for _, y := range ys {
					if x == v.K.X && y == v.K.Y {
						continue
					}
					if _, e = tx.Exec("UPDATE jobs SET state=0 WHERE q=? AND x=? AND y=? AND state=1", q, x, y); e != nil {
						return e
					}
				}
			}
		}
	}
	required, e := borderRows(tx, q, v.K)
	if e != nil {
		return e
	}
	state := 1
	for local, z := range required {
		if actual, ok := zs[local]; !ok || actual != z {
			state = 0
			break
		}
	}
	if v.Maximum > j.tolerance()+1e-6 || v.Angle < 10-1e-6 || len(p) > 65535 {
		return fmt.Errorf("階層基本の品質が未達: 誤差%.4f 角度%.4f 頂点%d", v.Maximum, v.Angle, len(p))
	}
	if _, e = tx.Exec("INSERT OR REPLACE INTO basic VALUES(?,?,?,?,?,?,?)", q, v.K.X, v.K.Y, len(p), len(f), body, v.Validity); e != nil {
		return e
	}
	if _, e = tx.Exec("INSERT OR REPLACE INTO basic_stats VALUES(?,?,?,'go-hierarchy',?,?,?)", q, v.K.X, v.K.Y, v.Maximum, v.Angle, v.Added); e != nil {
		return e
	}
	if _, e = tx.Exec("UPDATE jobs SET state=?,failure=NULL WHERE q=? AND x=? AND y=?", state, q, v.K.X, v.K.Y); e != nil {
		return e
	}
	return tx.Commit()
}
func (h *hierarchyPipeline) packLevel(q int) error {
	keys := []key{}
	rows, e := h.HB.Query("SELECT x,y FROM basic WHERE q=? ORDER BY y,x", q)
	if e != nil {
		return e
	}
	for rows.Next() {
		var k key
		if e = rows.Scan(&k.X, &k.Y); e != nil {
			rows.Close()
			return e
		}
		keys = append(keys, k)
	}
	if e = rows.Err(); e != nil {
		rows.Close()
		return e
	}
	rows.Close()
	for index, k := range keys {
		if h.R.stop() {
			return fmt.Errorf("階層圧縮を停止")
		}
		if index%128 == 0 {
			h.R.status("hierarchical-packing", q, index, len(keys), 0, 0)
		}
		if e = h.packTile(q, k); e != nil {
			return e
		}
	}
	_, e = h.Final.Exec("INSERT INTO metadata VALUES(?,?)", fmt.Sprintf("q%d-complete", q), fmt.Sprintf(`{"tiles":%d,"parentVerticesRetained":true,"sameLevelBordersVerified":true}`, len(keys)))
	return e
}
func (h *hierarchyPipeline) packTile(q int, k key) error {
	raw, e := h.finished(q, k)
	if e != nil {
		return e
	}
	j := job{q, k}
	p, f, e := decodeMesh(raw, j.n())
	if e != nil {
		return e
	}
	zs := meshHeights(raw, p)
	own := map[point]int{}
	for i, v := range p {
		own[v] = i
	}
	inherited := []int{}
	refs := []uint16{}
	parentQ := q + 1
	pk := key{floorDiv(k.X, 2), floorDiv(k.Y, 2)}
	stride := int64(j.n() - 1)
	if q < h.R.O.MaxQ {
		pp, e := restoreHierarchyVertices(h.Final, parentQ, pk)
		if e != nil {
			return e
		}
		for parentIndex, v := range pp {
			local := point{v.P.X - (k.X-pk.X*2)*stride, v.P.Y - (k.Y-pk.Y*2)*stride}
			if local.X < 0 || local.Y < 0 || local.X > stride || local.Y > stride {
				continue
			}
			i, ok := own[local]
			if !ok || zs[local] != v.Z {
				return fmt.Errorf("圧縮時の親頂点の欠損・標高変更 Q%d %v", q, k)
			}
			inherited = append(inherited, i)
			refs = append(refs, uint16(parentIndex))
		}
	} else {
		parentQ = -1
		pk = key{}
	}
	used := map[int]bool{}
	order := append([]int(nil), inherited...)
	for _, i := range inherited {
		used[i] = true
	}
	for i := range p {
		if !used[i] {
			order = append(order, i)
		}
	}
	if len(order) > 65535 {
		return fmt.Errorf("圧縮の頂点上限超過")
	}
	remap := make([]int, len(order))
	for i, old := range order {
		remap[old] = i
	}
	vertices := bytes.NewBuffer(nil)
	reference := bytes.NewBuffer(nil)
	for _, v := range refs {
		binary.Write(reference, le, v)
	}
	for _, old := range order[len(inherited):] {
		v := p[old]
		z := math.Round(zs[v] * 4)
		if z < -32768 || z > 32767 || z/4 != zs[v] {
			return fmt.Errorf("高さのint16量子化が不適合")
		}
		binary.Write(vertices, le, uint16(v.X))
		binary.Write(vertices, le, uint16(v.Y))
		binary.Write(vertices, le, int16(z))
	}
	remapped := make([]face, len(f))
	for i, v := range f {
		remapped[i] = face{remap[v[0]], remap[v[1]], remap[v[2]]}
	}
	encoded, e := encodeTFC(remapped)
	if e != nil {
		return e
	}
	decoded, e := decodeTFC(encoded, len(f), len(p))
	if e != nil {
		return e
	}
	if !sameOrientedFaces(remapped, decoded) {
		return fmt.Errorf("階層TFC1の復元不一致")
	}
	// 全境界点を端点順で保存し、混在LODの接続処理から参照可能にする。
	boundary := bytes.NewBuffer(nil)
	for side := 0; side < 4; side++ {
		ids := []int{}
		for old, v := range p {
			if (side == 0 && v.Y == 0) || (side == 1 && v.X == stride) || (side == 2 && v.Y == stride) || (side == 3 && v.X == 0) {
				ids = append(ids, old)
			}
		}
		sort.Slice(ids, func(a, b int) bool {
			pa, pb := p[ids[a]], p[ids[b]]
			if side == 0 || side == 2 {
				return pa.X < pb.X
			}
			return pa.Y < pb.Y
		})
		binary.Write(boundary, le, uint16(len(ids)))
		for _, old := range ids {
			binary.Write(boundary, le, uint16(remap[old]))
		}
	}
	ordered := make([]hierarchyVertex, len(order))
	for i, old := range order {
		ordered[i] = hierarchyVertex{p[old], zs[p[old]]}
	}
	hash := hierarchyDigest(ordered, remapped)
	_, e = h.Final.Exec("INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)", q, k.X, k.Y, parentQ, pk.X, pk.Y, len(p), len(f), len(inherited), reference.Bytes(), vertices.Bytes(), encoded, boundary.Bytes(), hash)
	if e != nil {
		return e
	}
	restored, e := restoreHierarchyVertices(h.Final, q, k)
	if e != nil {
		return e
	}
	if len(restored) != len(ordered) {
		return fmt.Errorf("階層頂点の復元数不一致")
	}
	for i, v := range restored {
		if v != ordered[i] {
			return fmt.Errorf("階層頂点の復元値不一致")
		}
	}
	if hierarchyDigest(restored, decoded) != hash {
		return fmt.Errorf("階層BLOBの復元ハッシュ不一致")
	}
	return nil
}

type hierarchyVertex struct {
	P point
	Z float64
}

func restoreHierarchyVertices(db *sql.DB, q int, k key) ([]hierarchyVertex, error) {
	if q < 4 || q > 12 {
		return nil, fmt.Errorf("親レベルが範囲外")
	}
	var parentQ, nv, nf, ni int
	var pk key
	var refs, vertices []byte
	if e := db.QueryRow("SELECT parent_q,parent_x,parent_y,vertices,triangles,inherited,parent_refs,new_vertices FROM tiles WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&parentQ, &pk.X, &pk.Y, &nv, &nf, &ni, &refs, &vertices); e != nil {
		return nil, e
	}
	if nv < 4 || nv > 65535 || ni < 0 || ni > nv || len(refs) != ni*2 || len(vertices) != (nv-ni)*6 {
		return nil, fmt.Errorf("階層頂点BLOBの長さ不正")
	}
	out := make([]hierarchyVertex, 0, nv)
	stride := int64(job{Q: q}.n() - 1)
	if parentQ >= 0 {
		if parentQ != q+1 || pk.X != floorDiv(k.X, 2) || pk.Y != floorDiv(k.Y, 2) {
			return nil, fmt.Errorf("階層の親参照不正")
		}
		parent, e := restoreHierarchyVertices(db, parentQ, pk)
		if e != nil {
			return nil, e
		}
		previous := -1
		for i := 0; i < ni; i++ {
			id := int(le.Uint16(refs[i*2:]))
			if id >= len(parent) || id <= previous {
				return nil, fmt.Errorf("親の頂点番号不正")
			}
			previous = id
			v := parent[id]
			v.P.X -= (k.X - pk.X*2) * stride
			v.P.Y -= (k.Y - pk.Y*2) * stride
			out = append(out, v)
		}
	} else if ni != 0 {
		return nil, fmt.Errorf("最粗レベルに親参照")
	}
	for i := 0; i < len(vertices); i += 6 {
		out = append(out, hierarchyVertex{point{int64(le.Uint16(vertices[i:])), int64(le.Uint16(vertices[i+2:]))}, float64(int16(le.Uint16(vertices[i+4:]))) / 4})
	}
	selected := map[point]bool{}
	for _, v := range out {
		if v.P.X < 0 || v.P.Y < 0 || v.P.X > stride || v.P.Y > stride || selected[v.P] {
			return nil, fmt.Errorf("階層復元座標が格子外・重複")
		}
		selected[v.P] = true
	}
	return out, nil
}
func canonicalHierarchyFaces(input []face) []face {
	out := append([]face(nil), input...)
	for i, v := range out {
		j := 0
		if v[1] < v[j] {
			j = 1
		}
		if v[2] < v[j] {
			j = 2
		}
		out[i] = face{v[j], v[(j+1)%3], v[(j+2)%3]}
	}
	sort.Slice(out, func(i, j int) bool {
		for k := 0; k < 3; k++ {
			if out[i][k] != out[j][k] {
				return out[i][k] < out[j][k]
			}
		}
		return false
	})
	return out
}
func hierarchyDigest(p []hierarchyVertex, f []face) string {
	b := bytes.NewBuffer(nil)
	for _, v := range p {
		binary.Write(b, le, uint16(v.P.X))
		binary.Write(b, le, uint16(v.P.Y))
		binary.Write(b, le, int16(math.Round(v.Z*4)))
	}
	for _, v := range canonicalHierarchyFaces(f) {
		for _, id := range v {
			binary.Write(b, le, uint16(id))
		}
	}
	hash := sha256.Sum256(b.Bytes())
	return hex.EncodeToString(hash[:])
}
func sameOrientedFaces(a, b []face) bool {
	if len(a) != len(b) {
		return false
	}
	normalize := func(input []face) []face {
		out := append([]face(nil), input...)
		for i, v := range out {
			j := 0
			if v[1] < v[j] {
				j = 1
			}
			if v[2] < v[j] {
				j = 2
			}
			out[i] = face{v[j], v[(j+1)%3], v[(j+2)%3]}
		}
		sort.Slice(out, func(i, j int) bool {
			for k := 0; k < 3; k++ {
				if out[i][k] != out[j][k] {
					return out[i][k] < out[j][k]
				}
			}
			return false
		})
		return out
	}
	aa, bb := normalize(a), normalize(b)
	for i := range aa {
		if aa[i] != bb[i] {
			return false
		}
	}
	return true
}
