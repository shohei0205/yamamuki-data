package main

import (
	"bytes"
	"container/heap"
	"context"
	"database/sql"
	"encoding/binary"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"github.com/klauspost/compress/zstd"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"sort"
	"strings"
	"sync"
	"time"
)

type options struct {
	Hierarchical bool
	HierarchyOutput, FinalOutput string
	MinFreeGiB                        float64
	Base, Output, Cache, Stop, Source string
	Workers, Limit, MinQ, MaxQ        int
	Free                              float64
	CacheGiB                          float64
}
type runner struct {
	O               options
	Base, DB, Cache *sql.DB
	Start           time.Time
}

func main() {
	o := options{}
	flag.BoolVar(&o.Hierarchical,"hierarchical",false,"親の確定後に子を生成する階層LOD")
	flag.StringVar(&o.HierarchyOutput,"hierarchy-output","","階層基本メッシュの別出力")
	flag.StringVar(&o.FinalOutput,"final-output","","頂点差分とTFC1の別出力")
	flag.StringVar(&o.Base, "base-input", "", "基本メッシュSQLite")
	flag.StringVar(&o.Output, "output", "", "追加出力SQLite（基本と別ファイル）")
	flag.StringVar(&o.Cache, "field-cache-input", "", "読み取り専用の標高重みキャッシュ")
	flag.StringVar(&o.Source, "source", "build/terrain-lod.sqlite3", "元標高SQLite")
	flag.StringVar(&o.Stop, "stop-file", "", "停止要求ファイル")
	flag.IntVar(&o.Workers, "workers", max(1, runtime.NumCPU()/2), "並列数")
	flag.IntVar(&o.Limit, "limit", -1, "試験用の今回の追加点数上限")
	flag.IntVar(&o.MinQ, "min-q", 4, "開始レベル")
	flag.IntVar(&o.MaxQ, "max-q", 12, "最終レベル")
	flag.Float64Var(&o.Free, "density-free-vertices", 4096, "減点しない周辺頂点数")
	flag.Float64Var(&o.CacheGiB, "cache-gib", 8, "常駐メッシュの推定メモリー枠")
	flag.Float64Var(&o.MinFreeGiB, "min-free-gib", 8, "最低空き容量GiB")
	flag.Parse()
	if e := execute(o); e != nil {
		fmt.Fprintln(os.Stderr, "生成失敗:", e)
		os.Exit(1)
	}
}
func stem(p string) string { return strings.TrimSuffix(p, filepath.Ext(p)) }
func jsonFile(p string, v any) error {
	b, e := json.MarshalIndent(v, "", "  ")
	if e != nil {
		return e
	}
	tmp := p + ".tmp"
	if e = os.WriteFile(tmp, b, 0600); e != nil {
		return e
	}
	return os.Rename(tmp, p)
}
func (r *runner) status(stage string, q, prepared, total int, used, budget uint64) error {
	return jsonFile(stem(r.O.Output)+".progress.json", map[string]any{"engine": "go", "stage": stage, "q": q, "preparedTiles": prepared, "totalTiles": total, "addedVertices": used, "budget": budget, "pid": os.Getpid(), "updated": float64(time.Now().UnixNano()) / 1e9, "elapsedSeconds": time.Since(r.Start).Seconds(), "workers": r.O.Workers, "densityFreeVertices": r.O.Free, "complete": false, "distributable": false})
}
func (r *runner) stop() bool {
	if r.O.Stop == "" {
		return false
	}
	_, e := os.Stat(r.O.Stop)
	return e == nil
}
func execute(o options) error {
	if o.Hierarchical {return executeHierarchy(o)}
	if o.Base == "" || o.Output == "" || o.Workers < 1 || !finite(o.Free) || o.Free < 0 || !finite(o.CacheGiB) || o.CacheGiB <= 0 || !finite(o.MinFreeGiB) || o.MinFreeGiB < 0 || o.MinQ < 4 || o.MaxQ > 12 || o.MinQ > o.MaxQ {
		return fmt.Errorf("引数が不正です")
	}
	b, _ := filepath.Abs(o.Base)
	out, _ := filepath.Abs(o.Output)
	if strings.EqualFold(b, out) {
		return fmt.Errorf("基本と追加出力は別ファイルにしてください")
	}
	if e := os.MkdirAll(filepath.Dir(out), 0700); e != nil {
		return e
	}
	unlock, e := lockOutput(stem(out) + ".go-lock")
	if e != nil {
		return e
	}
	defer unlock()
	debug.SetMemoryLimit(int64(min(float64(availableMemory())*.65,(o.CacheGiB+2)*1073741824)))
	r := runner{O: o, Start: time.Now()}
	r.Base, e = openDB(o.Base, true)
	if e != nil {
		return e
	}
	defer r.Base.Close()
	var pending int
	if e = r.Base.QueryRow("SELECT count(*) FROM jobs WHERE state NOT IN (1,2)").Scan(&pending); e != nil {
		return e
	}
	if pending > 0 {
		return fmt.Errorf("基本メッシュが未完了です")
	}
	var manifest string
	if e = r.Base.QueryRow("SELECT value FROM metadata WHERE key='manifest'").Scan(&manifest); e != nil {
		return e
	}
	var m map[string]any
	if e = json.Unmarshal([]byte(manifest), &m); e != nil {
		return e
	}
	if path, ok := m["source"].(string); ok {
		r.O.Source = path
	}
	var sourceInfo map[string]json.RawMessage
	json.Unmarshal([]byte(manifest), &sourceInfo)
	info, err := os.Stat(r.O.Source)
	if err != nil {
		return err
	}
	var sourceSize, sourceTime int64
	json.Unmarshal(sourceInfo["sourceSize"], &sourceSize)
	json.Unmarshal(sourceInfo["sourceMtimeNs"], &sourceTime)
	if info.Size() != sourceSize || info.ModTime().UnixNano() != sourceTime {
		return fmt.Errorf("元標高が基本生成時と異なります")
	}
	r.DB, e = openDB(o.Output, false)
	if e != nil {
		return e
	}
	defer r.DB.Close()
	var tables int
	if e = r.DB.QueryRow("SELECT count(*) FROM sqlite_master WHERE type='table'").Scan(&tables); e != nil {
		return e
	}
	config, _ := json.Marshal(map[string]any{"format": "YQ-additional-go-v1", "basePath": b, "manifest": json.RawMessage(manifest), "densityFreeVertices": o.Free, "additionalFraction": .2, "minQ": o.MinQ, "maxQ": o.MaxQ, "minimumAngleDegrees": 10, "policy": "global-pool-go-v1"})
	if tables > 0 {
		var old string
		if e = r.DB.QueryRow("SELECT value FROM metadata WHERE key='go_additional_config'").Scan(&old); e != nil || old != string(config) {
			return fmt.Errorf("出力DBの方式または基本入力が違います。別の出力名を指定してください")
		}
	}
	_, e = r.DB.Exec(`PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
 CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);
 CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS additional(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,added_vertex_ids BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS working(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,ids BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS candidates(q INTEGER,x INTEGER,y INTEGER,score REAL,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS trace(q INTEGER,seq INTEGER,x INTEGER,y INTEGER,sample INTEGER,priority REAL,PRIMARY KEY(q,seq));
 CREATE TABLE IF NOT EXISTS fields(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));
 CREATE TABLE IF NOT EXISTS fields_fast(q INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(q,x,y));`)
	if e != nil {
		return e
	}
	_, e = r.DB.Exec("INSERT OR IGNORE INTO metadata VALUES('go_additional_config',?)", string(config))
	if e != nil {
		return e
	}
	r.DB.Exec("INSERT OR IGNORE INTO metadata VALUES('complete','false'),('distributable','false')")
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
			Base struct {
				Manifest json.RawMessage `json:"manifest"`
			} `json:"base"`
		}
		if e = json.Unmarshal([]byte(c), &v); e != nil {
			return e
		}
		var a, cfg any
		json.Unmarshal(v.Base.Manifest, &a)
		json.Unmarshal([]byte(manifest), &cfg)
		if !sameFieldSource(a, cfg) {
			return fmt.Errorf("標高キャッシュの生成元が違います")
		}
	}
	for q := o.MinQ; q <= o.MaxQ; q++ {
		var done int
		r.DB.QueryRow("SELECT count(*) FROM metadata WHERE key=?", fmt.Sprintf("q%d-complete", q)).Scan(&done)
		if done > 0 {
			continue
		}
		stopped, e := r.level(q)
		if e != nil {
			return e
		}
		if stopped {
			return nil
		}
	}
	_, e = r.DB.Exec("INSERT OR REPLACE INTO metadata VALUES('stage','additional-ready-for-boundary')")
	if e != nil {
		return e
	}
	return r.status("additional-ready-for-boundary", o.MaxQ, 0, 0, 0, 0)
}

type resident struct {
	B       *builder
	Base    int
	Dirty   bool
	Version uint64
}

func (r *runner) load(q int, k key, d *density, query queryer) (*resident, error) {
	j := job{q, k}
	var base int
	var blob []byte
	if e := r.Base.QueryRow("SELECT vertices,mesh FROM basic WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&base, &blob); e != nil {
		return nil, e
	}
	var working []byte
	e := query.QueryRow("SELECT mesh FROM working WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&working)
	if e != nil && e != sql.ErrNoRows {
		return nil, e
	}
	if e == nil {
		blob = working
	}
	raw, e := unzip(blob)
	if e != nil {
		return nil, e
	}
	p, f, e := decodeMesh(raw, j.n())
	if e != nil {
		return nil, e
	}
	grid, limits, e := fieldData(query, r.Cache, j)
	if errors.Is(e, errNoFields) {
		src, err := newSource(r.O.Source)
		if err != nil {
			return nil, err
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
		enc, err := zstd.NewWriter(nil, zstd.WithEncoderLevel(zstd.SpeedFastest), zstd.WithEncoderConcurrency(1))
		if err != nil {
			return nil, err
		}
		blob := enc.EncodeAll(raw, nil)
		enc.Close()
		writer := query.(interface {
			Exec(string, ...any) (sql.Result, error)
		})
		if _, e = writer.Exec("INSERT OR IGNORE INTO fields_fast VALUES(?,?,?,?)", j.Q, k.X, k.Y, blob); e != nil {
			return nil, e
		}
	} else if e != nil {
		return nil, e
	}
	if r.O.Hierarchical {for i,v:=range p {at:=int(v.Y)*j.n()+int(v.X);z:=float64(math.Float32frombits(le.Uint32(raw[12+i*12+8:])));if math.Abs(grid[at]-z)>.250001{return nil,fmt.Errorf("階層の固定頂点と元標高が不一致")};grid[at]=z}}
	b, e := newBuilder(j.n(), grid, limits, p, f, d.tile(k), r.O.Free, j.tolerance())
	if e != nil {
		return nil, e
	}
	return &resident{B: b, Base: base}, nil
}
func flush(tx *sql.Tx, q int, k key, t *resident) error {
	if !t.Dirty {
		return nil
	}
	blob, e := zipBytes(t.B.raw(), 1)
	if e != nil {
		return e
	}
	var ids bytes.Buffer
	for i := t.Base; i < len(t.B.Points); i++ {
		binary.Write(&ids, le, uint32(i))
	}
	idb, e := zipBytes(ids.Bytes(), 6)
	if e != nil {
		return e
	}
	_, e = tx.Exec("INSERT OR REPLACE INTO working VALUES(?,?,?,?,?,?,?)", q, k.X, k.Y, len(t.B.Points), len(t.B.Tris), blob, idb)
	if e == nil {
		t.Dirty = false
	}
	return e
}
func (r *runner) level(q int) (bool, error) {
	j := job{Q: q}
	d := newDensity(j)
	keys := []key{}
	shared := map[point]bool{}
	var unique uint64
	r.status("indexing-density", q, 0, 0, 0, 0)
	rows, e := r.Base.Query("SELECT x,y,mesh FROM basic WHERE q=? ORDER BY y,x", q)
	if e != nil {
		return false, e
	}
	for rows.Next() {
		var k key
		var blob []byte
		if e = rows.Scan(&k.X, &k.Y, &blob); e != nil {
			rows.Close()
			return false, e
		}
		raw, e := unzip(blob)
		if e != nil {
			rows.Close()
			return false, e
		}
		p, _, e := decodeMesh(raw, j.n())
		if e != nil {
			rows.Close()
			return false, e
		}
		unique += d.addBasicTile(k,p,shared)
		keys = append(keys, k)
        if len(keys)%4096==0{fmt.Printf("全国密度 Q%d %dタイル\n",q,len(keys));r.status("indexing-density",q,len(keys),0,0,0)}
        if r.stop(){rows.Close();r.status("paused",q,len(keys),0,0,0);return true,nil}
	}
	if e = rows.Err(); e != nil {
		rows.Close()
		return false, e
	}
	rows.Close()
	budget := unique / 5
	versions := map[key]uint64{}
	var used uint64
	rows, e = r.DB.Query("SELECT x,y,sample FROM trace WHERE q=? ORDER BY seq", q)
	if e != nil {
		return false, e
	}
	for rows.Next() {
		var k key
		var sample int
		if e = rows.Scan(&k.X, &k.Y, &sample); e != nil {
			rows.Close()
			return false, e
		}
		d.add(d.world(k, sample))
		used++
		for y := int64(-1); y <= 1; y++ {
			for x := int64(-1); x <= 1; x++ {
				versions[key{k.X + x, k.Y + y}]++
			}
		}
	}
	if e = rows.Err(); e != nil {
		rows.Close()
		return false, e
	}
	rows.Close()
	h := candidateHeap{}
	todo := []key{}
	prepared := 0
	for _, k := range keys {
		var score float64
		e = r.DB.QueryRow("SELECT score FROM candidates WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&score)
		if e == nil {
			h.add(k, score)
			prepared++
			continue
		}
		if e != sql.ErrNoRows {
			return false, e
		}
		var bound float64
		e = r.Base.QueryRow("SELECT max_error FROM basic_stats WHERE q=? AND x=? AND y=?", q, k.X, k.Y).Scan(&bound)
		if e == nil && bound*1.5/j.tolerance() <= .1 {
			_, e = r.DB.Exec("INSERT INTO candidates VALUES(?,?,?,?)", q, k.X, k.Y, bound*1.5/j.tolerance())
			if e != nil {
				return false, e
			}
			prepared++
			continue
		}
		todo = append(todo, k)
	}
	e = nil
	r.status("preparing-candidates",q,prepared,len(keys),used,budget)
	type result struct {
		K     key
		Score float64
		Err   error
	}
	prepareWorkers := max(1, min(r.O.Workers, int(availableMemory()/(uint64(j.n())*uint64(j.n())*40+512*1024*1024))))
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	tasks := make(chan key)
	results := make(chan result, r.O.Workers)
	var wg sync.WaitGroup
	var cancelled bool
	for w := 0; w < min(prepareWorkers, len(todo)); w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for k := range tasks {
				if ctx.Err() != nil {
					return
				}
				t, e := r.load(q, k, d, r.DB)
				s := 0.
				if e == nil {
					s = t.B.next().Score
				}
				results <- result{k, s, e}
			}
		}()
	}
	go func() {
	sendLoop:
		for _, k := range todo {
			select {
			case tasks <- k:
			case <-ctx.Done():
				break sendLoop
			}
		}
		close(tasks)
		wg.Wait()
		close(results)
	}()
	for v := range results {
		if v.Err != nil {
			if e == nil {
				e = v.Err
			}
			cancelled = true
			cancel()
			continue
		}
		if cancelled {
			continue
		}
		_, err := r.DB.Exec("INSERT INTO candidates VALUES(?,?,?,?)", q, v.K.X, v.K.Y, v.Score)
		if err != nil {
			e = err
			cancelled = true
			cancel()
			continue
		}
		h.add(v.K, v.Score)
		prepared++
		if prepared%16 == 0 || prepared == len(keys) {
			if err := r.checkDisk(); err != nil {
				e = err
				cancelled = true
				cancel()
				continue
			}
			fmt.Printf("全国追加候補 Q%d %d/%d 並列%d\n", q, prepared, len(keys), prepareWorkers)
			r.status("preparing-candidates", q, prepared, len(keys), used, budget)
		}
		if r.stop() {
			cancelled = true
			cancel()
		}
	}
	if e != nil {
		return false, e
	}
	if cancelled {
		r.status("paused", q, prepared, len(keys), used, budget)
		return true, nil
	}
	cache := map[key]*resident{}
	cacheBytes := min(uint64(r.O.CacheGiB*1073741824), availableMemory()*2/3)
	bytesPerSample:=24
 if j.n()<=2049 {bytesPerSample=48}
 capacity := max(1, int(cacheBytes)/(j.n()*j.n()*bytesPerSample+1048576))
	tx, e := r.DB.Begin()
	if e != nil {
		return false, e
	}
	defer func() { tx.Rollback() }()
	initial := used
	started := time.Now()
	var prioritySeconds, loadSeconds, insertSeconds, saveSeconds float64
	since := 0
	save := func() error {
		for k, t := range cache {
			if err := flush(tx, q, k, t); err != nil {
				return err
			}
		}
		return nil
	}
	for used < budget && len(h) > 0 {
		if r.stop() || (r.O.Limit >= 0 && used-initial >= uint64(r.O.Limit)) {
			if e = save(); e != nil {
				return false, e
			}
			if e = tx.Commit(); e != nil {
				return false, e
			}
			r.status("paused", q, len(keys), len(keys), used, budget)
			return true, nil
		}
		top := heap.Pop(&h).(candidate)
		if top.Score <= .1 {
			break
		}
		at := time.Now()
		t := cache[top.K]
		if t == nil {
			if len(cache) >= capacity {
				scores := map[key]float64{}
				for _, c := range h {
					scores[c.K] = c.Score
				}
				ks := make([]key, 0, len(cache))
				for k := range cache {
					ks = append(ks, k)
				}
				sort.Slice(ks, func(i, j int) bool {
					a, b := ks[i], ks[j]
					if scores[a] != scores[b] {
						return scores[a] < scores[b]
					}
					if a.X != b.X {
						return a.X > b.X
					}
					return a.Y > b.Y
				})
				if e = flush(tx, q, ks[0], cache[ks[0]]); e != nil {
					return false, e
				}
				delete(cache, ks[0])
			}
			t, e = r.load(q, top.K, d, tx)
			if e != nil {
				return false, e
			}
			t.Version = versions[top.K]
			cache[top.K] = t
		}
		loadSeconds += time.Since(at).Seconds()
		at = time.Now()
		if t.Version != versions[top.K] {
			t.B.setDensity(d.tile(top.K))
			t.Version = versions[top.K]
		}
		next := t.B.next()
		prioritySeconds += time.Since(at).Seconds()
		if next.Score < top.Score-1e-12 {
			_, e = tx.Exec("UPDATE candidates SET score=? WHERE q=? AND x=? AND y=?", next.Score, q, top.K.X, top.K.Y)
			if e != nil {
				return false, e
			}
			h.add(top.K, next.Score)
			continue
		}
		if len(t.B.Points) >= 65536 {
			continue
		}
		at = time.Now()
		if e = t.B.step(next); e != nil {
			return false, e
		}
		insertSeconds += time.Since(at).Seconds()
		t.Dirty = true
		used++
		d.add(d.world(top.K, next.Sample))
		for y := int64(-1); y <= 1; y++ {
			for x := int64(-1); x <= 1; x++ {
				versions[key{top.K.X + x, top.K.Y + y}]++
			}
		}
		at = time.Now()
		t.B.setDensity(d.tile(top.K))
		t.Version = versions[top.K]
		after := t.B.next()
		prioritySeconds += time.Since(at).Seconds()
		if _, e = tx.Exec("INSERT INTO trace VALUES(?,?,?,?,?,?)", q, used, top.K.X, top.K.Y, next.Sample, next.Score); e != nil {
			return false, e
		}
		if _, e = tx.Exec("UPDATE candidates SET score=? WHERE q=? AND x=? AND y=?", after.Score, q, top.K.X, top.K.Y); e != nil {
			return false, e
		}
		h.add(top.K, after.Score)
		since++
		if since >= 128 {
			if e = r.checkDisk(); e != nil {
				return false, e
			}
			at = time.Now()
			if e = save(); e != nil {
				return false, e
			}
			if e = tx.Commit(); e != nil {
				return false, e
			}
			tx, e = r.DB.Begin()
			if e != nil {
				return false, e
			}
			saveSeconds += time.Since(at).Seconds()
			since = 0
			r.status("adding-vertices", q, len(keys), len(keys), used, budget)
			jsonFile(stem(r.O.Output)+".profile.json", map[string]any{"q": q, "inserted": used - initial, "elapsedSeconds": time.Since(started).Seconds(), "prioritySeconds": prioritySeconds, "loadSeconds": loadSeconds, "stepSeconds": insertSeconds, "flushSeconds": saveSeconds})
			fmt.Printf("全国追加 Q%d %d/%d\n", q, used, budget)
		}
	}
	if e = save(); e != nil {
		return false, e
	}
	rows, e = tx.Query("SELECT x,y,vertices,triangles,mesh,ids FROM working WHERE q=? ORDER BY y,x", q)
	if e != nil {
		return false, e
	}
	type published struct {
		k         key
		nv, nf    int
		mesh, ids []byte
	}
	all := []published{}
	for rows.Next() {
		var p published
		if e = rows.Scan(&p.k.X, &p.k.Y, &p.nv, &p.nf, &p.mesh, &p.ids); e != nil {
			rows.Close()
			return false, e
		}
		all = append(all, p)
	}
	if e = rows.Err(); e != nil {
		rows.Close()
		return false, e
	}
	rows.Close()
	for _, p := range all {
		raw, err := unzip(p.mesh)
		if err != nil {
			return false, err
		}
		mesh, err := zipBytes(raw, 6)
		if err != nil {
			return false, err
		}
		if _, e = tx.Exec("INSERT OR REPLACE INTO additional VALUES(?,?,?,?,?,?,?)", q, p.k.X, p.k.Y, p.nv, p.nf, mesh, p.ids); e != nil {
			return false, e
		}
	}
	for _, k := range keys {
		if _, e = tx.Exec("INSERT OR REPLACE INTO jobs(q,x,y,state) VALUES(?,?,?,CASE WHEN EXISTS(SELECT 1 FROM additional WHERE q=? AND x=? AND y=?) THEN 1 ELSE 2 END)", q, k.X, k.Y, q, k.X, k.Y); e != nil {
			return false, e
		}
	}
	value, _ := json.Marshal(map[string]any{"uniqueBasicVertices": unique, "budget": budget, "used": used})
	if _, e = tx.Exec("INSERT INTO metadata VALUES(?,?)", fmt.Sprintf("q%d-complete", q), string(value)); e != nil {
		return false, e
	}
	if e = tx.Commit(); e != nil {
		return false, e
	}
	fmt.Printf("全国追加 Q%d 完了 %d/%d\n", q, used, budget)
	return false, nil
}

// 丸めが変わり得る派生統計値を除き、標高取得と重み計算の入力を比較する。
func sameFieldSource(a, b any) bool {
	x, ok := a.(map[string]any)
	if !ok {
		return false
	}
	y, ok := b.(map[string]any)
	if !ok {
		return false
	}
	for _, k := range []string{"source", "sourceSize", "sourceMtimeNs"} {
		if fmt.Sprint(x[k]) != fmt.Sprint(y[k]) {
			return false
		}
	}
	la, ok := x["levels"].([]any)
	if !ok {
		return false
	}
	lb, ok := y["levels"].([]any)
	if !ok || len(la) != len(lb) {
		return false
	}
	for i := range la {
		u := la[i].(map[string]any)
		v := lb[i].(map[string]any)
		for _, k := range []string{"level", "widthMeters", "maxErrorMeters", "sampleSpacingMeters"} {
			if u[k] != v[k] {
				return false
			}
		}
	}
	return true
}

func (r *runner) checkDisk() error {
	free, e := availableDisk(r.O.Output)
	if e != nil {
		return e
	}
	if float64(free) < r.O.MinFreeGiB*1073741824 {
		return fmt.Errorf("空き容量が不足しています: %.2f GiB", float64(free)/1073741824)
	}
	return nil
}
