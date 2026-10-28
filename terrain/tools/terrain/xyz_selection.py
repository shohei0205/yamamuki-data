"""カメラの画角と描画予算から全国XYZタイルを選ぶ。"""
import heapq
import math
import sqlite3

WORLD = 40075016.68557849
UNIT = WORLD / 268435456


class TileCatalog:
    def __init__(self, path):
        with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
            self.rows = {(z, x, y): (nv, nf, body, size) for z, x, y, nv, nf, body, size in db.execute('SELECT z,tile_x,tile_y,vertex_count,triangle_count,body_triangle_count,length(vertices)+length(triangles) FROM tiles')}
        self.roots = sorted(k for k in self.rows if k[0] == 6)

    def children(self, key):
        z, x, y = key
        return [(z+1, 2*x+dx, 2*y+dy) for dy in range(2) for dx in range(2) if (z+1, 2*x+dx, 2*y+dy) in self.rows]

    def select(self, camera):
        origin = camera['origin']
        target = camera['target']
        distance = float(camera['distance'])
        yaw, pitch = float(camera['yaw']), float(camera['pitch'])
        width, height = map(float, camera['viewport'])
        maximum = int(camera['maximum'])
        mode = camera.get('mode', 'auto')
        divisions = int(camera.get('divisions', 1))
        if not (6 <= maximum <= 14 and mode in ('auto', 'fixed') and divisions in (1,2,4,8) and 100 <= distance <= 20000000 and 0.02 <= pitch <= 1.57 and 1 <= width <= 20000 and 1 <= height <= 20000):
            raise ValueError('カメラ設定が不正です')
        if len(origin) != 2 or len(target) != 3 or not all(math.isfinite(float(v)) for v in [*origin, *target, distance, yaw, pitch, width, height]):
            raise ValueError('カメラ座標が不正です')
        back = [math.cos(pitch)*math.sin(yaw), math.sin(pitch), math.cos(pitch)*math.cos(yaw)]
        right = [math.cos(yaw), 0, -math.sin(yaw)]
        up = [-math.sin(pitch)*math.sin(yaw), math.cos(pitch), -math.sin(pitch)*math.cos(yaw)]
        eye = [target[i] + distance*back[i] for i in range(3)]
        tangent = math.tan(math.pi/8)
        aspect = width / height
        projected = {}

        def view(key):
            if key in projected:
                return projected[key]
            z, x, y = key
            span = WORLD / 2**z
            center = [(x+.5)*span-origin[0]*UNIT, 2000, (y+.5)*span-origin[1]*UNIT]
            delta = [center[i]-eye[i] for i in range(3)]
            depth = -sum(delta[i]*back[i] for i in range(3))
            def support(vector):
                return span*.5*(abs(vector[0])+abs(vector[2]))+2100*abs(vector[1])
            radius = support(back)
            visible = depth+radius > max(1, distance/10000)
            for axis, slope in ((right,tangent*aspect*1.15),(up,tangent*1.15)):
                for sign in (-1,1):
                    plane = [sign*axis[i]+back[i]*slope for i in range(3)]
                    visible = visible and sum(delta[i]*plane[i] for i in range(3)) <= support(plane)
            # 近い地表は細かく、遠景は粗くする。回転方向ではなく投影誤差で決める。
            near = max(100, depth-radius*.5)
            error = 12.5*2**(14-z)*height/(2*tangent*near)
            projected[key] = (visible, error)
            return projected[key]

        if mode == 'fixed':
            selected = {key for key in self.rows if key[0] == maximum and view(key)[0]}
            # 固定モードではレベルを落とさず、カメラ側で範囲を狭める。
            if len(selected) > 256 or sum(self.rows[k][1] for k in selected) > 500000:
                return {'tiles': [], 'retryDistance': distance*.65, 'mode': mode, 'maximum': maximum}
        else:
            selected = {key for key in self.roots if view(key)[0]}
            triangles = sum(self.rows[k][1] for k in selected)
            budget = max(triangles, 350000)
            # 曲面の分割数で元データのLODを粗くしない。GPU側の適用分割数で予算を守る。
            queue = []
            for key in selected:
                heapq.heappush(queue, (-view(key)[1], key))
            while queue:
                negative, key = heapq.heappop(queue)
                if key not in selected or key[0] >= maximum or -negative <= 3:
                    continue
                children = [child for child in self.children(key) if view(child)[0]]
                if not children:
                    continue
                cost = sum(self.rows[k][1] for k in children)-self.rows[key][1]
                if triangles+cost > budget or len(selected)-1+len(children) > 192:
                    continue
                selected.remove(key)
                selected.update(children)
                triangles += cost
                for child in children:
                    heapq.heappush(queue, (-view(child)[1], child))
            # 極端な隣接差を抑える。上限予算を超える方向へは細分化しない。
            while True:
                leaves = sorted(selected)
                changed = False
                for i, a in enumerate(leaves):
                    sa = 2**(14-a[0]); ax, ay = a[1]*sa, a[2]*sa
                    for b in leaves[i+1:]:
                        if abs(a[0]-b[0]) <= 2:
                            continue
                        sb = 2**(14-b[0]); bx, by = b[1]*sb, b[2]*sb
                        adjacent = ((ax+sa == bx or bx+sb == ax) and max(ay,by)<min(ay+sa,by+sb)) or ((ay+sa == by or by+sb == ay) and max(ax,bx)<min(ax+sa,bx+sb))
                        if not adjacent:
                            continue
                        coarse, fine = (a,b) if a[0]<b[0] else (b,a)
                        z = coarse[0]+2
                        ancestor = (z, fine[1]//2**(fine[0]-z), fine[2]//2**(fine[0]-z))
                        selected = {k for k in selected if not (k[0]>=z and k[1]//2**(k[0]-z)==ancestor[1] and k[2]//2**(k[0]-z)==ancestor[2])}
                        selected.add(ancestor)
                        changed = True
                        break
                    if changed:
                        break
                if not changed:
                    break
        counts = {}
        for z,x,y in selected:
            counts[str(z)] = counts.get(str(z),0)+1
        return {'tiles': sorted(selected), 'levelCounts': counts, 'mode': mode, 'maximum': maximum, 'sourceTriangles': sum(self.rows[k][1] for k in selected), 'screenErrorPixels': 3, 'tileBudget': 192 if mode == 'auto' else 256}
