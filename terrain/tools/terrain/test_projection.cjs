// 生成済み HTML の実際の頂点シェーダーを GPU で動かし、透視投影を検証する。
const assert = require('node:assert/strict');
const path = require('node:path');
const { pathToFileURL, fileURLToPath } = require('node:url');
const { chromium } = require('playwright');

async function main() {
  const browser = await chromium.launch({
    headless: true,
    channel: process.env.TERRAIN_BROWSER_CHANNEL || (process.platform === 'win32' ? 'msedge' : undefined),
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1200, height: 1000 } });
    const errors = [];
    const requests = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route(/^https?:/, route => { requests.push(route.request().url()); return route.abort(); });
    const url = pathToFileURL(path.resolve(process.argv[2] || 'build/terrain-yatsugatake/preview.html')).href;
    await page.goto(url);
    await page.waitForFunction(() => document.getElementById("stats").textContent.includes("三角形"));
    const restored = await page.evaluate(async () => {
      const field={...data.heightfield,shape:[3,3],step:2};
      const heights=new Float32Array([100,100,100,100,NaN,100,100,100,100]);
      const bytes=await new Response(new Blob([heights]).stream().pipeThrough(new CompressionStream("gzip"))).arrayBuffer();
      field.values=btoa(String.fromCharCode(...new Uint8Array(bytes)));
      return restoreTerrain(field);
    });
    assert.equal(restored.mesh.length,4);
    assert.equal(restored.indices.length,0,"間引いた頂点の間の欠損をふさがないこと");
    const normals = await page.evaluate(() => {
      const original = data.mesh;
      try {
        data.mesh = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 1]];
        return Array.from(terrainNormals([0, 1, 2, 1, 3, 2]));
      } finally {
        data.mesh = original;
      }
    });
    assert.ok(normals.every(Number.isFinite));
    assert.equal(normals[2], 1, '水平面の法線が上を向くこと');
    for (const i of [1, 2]) {
      const expected = [-1 / Math.sqrt(6), -1 / Math.sqrt(6), 2 / Math.sqrt(6)];
      expected.forEach((value, j) => assert.ok(Math.abs(normals[i * 3 + j] - value) < 1e-6,
        '共有頂点の法線が隣接する面を面積で重み付けして平均されること'));
    }
    const result = await page.evaluate(() => {
      const vertex = gl.getAttachedShaders(program).find(s => gl.getShaderParameter(s, gl.SHADER_TYPE) === gl.VERTEX_SHADER);
      const fragment = gl.getAttachedShaders(program).find(s => gl.getShaderParameter(s, gl.SHADER_TYPE) === gl.FRAGMENT_SHADER);
      // 計算式をコピーせず、本体のシェーダーに検証用の出力だけを足す。
      const source = gl.getShaderSource(vertex)
        .replace('out vec3 terrainPosition;', 'out vec3 terrainPosition;out vec4 clipPosition;')
        .replace(/gl_Position=(.*);/, 'gl_Position=$1;clipPosition=gl_Position;');
      const check = gl.createProgram();
      gl.attachShader(check, shader(gl.VERTEX_SHADER, source));
      gl.attachShader(check, fragment);
      gl.transformFeedbackVaryings(check, ['clipPosition'], gl.INTERLEAVED_ATTRIBS);
      gl.linkProgram(check);
      if (!gl.getProgramParameter(check, gl.LINK_STATUS)) throw Error(gl.getProgramInfoLog(check));
      gl.useProgram(check);
      const values = { bearing: 0, tilt: 0, focalLength: 1, aspect: 1, cameraDistance: 100, nearPlane: 10, farPlane: 200 };
      for (const [name, value] of Object.entries(values)) gl.uniform1f(gl.getUniformLocation(check, name), value);
      const points = [20, 10, 0, 20, 10, 50, 0, 0, 0, 0, 0, 90, 0, 0, -100];
      const input = buffer(points);
      const attribute = gl.getAttribLocation(check, 'position');
      gl.bindBuffer(gl.ARRAY_BUFFER, input);
      gl.enableVertexAttribArray(attribute);
      gl.vertexAttribPointer(attribute, 3, gl.FLOAT, false, 0, 0);
      const feedback = gl.createTransformFeedback();
      gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, feedback);
      const output = gl.createBuffer();
      gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER, output);
      gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER, 20 * 4, gl.STREAM_READ);
      gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER, 0, output);
      gl.enable(gl.RASTERIZER_DISCARD);
      gl.beginTransformFeedback(gl.POINTS);
      gl.drawArrays(gl.POINTS, 0, 5);
      gl.endTransformFeedback();
      gl.disable(gl.RASTERIZER_DISCARD);
      const projected = new Float32Array(20);
      gl.getBufferSubData(gl.TRANSFORM_FEEDBACK_BUFFER, 0, projected);
      return { clip: Array.from(projected), error: gl.getError() };
    });
    assert.equal(result.error, 0, '投影の検証で WebGL エラーがないこと');
    const clip = result.clip;
    assert.ok(clip.every(Number.isFinite), '投影値がすべて有限であること');
    const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-5, `${a} と ${b} が一致すること`);
    close(clip[0] / clip[3], 0.2);
    close(clip[4] / clip[7], 0.4);
    close(clip[5] / clip[7], 0.2);
    assert.ok(clip[4] / clip[7] > clip[0] / clip[3], '真上からでも高い地点ほど外側へ大きく投影されること');
    close(clip[8], 0);
    close(clip[9], 0);
    close(clip[14] / clip[15], -1);
    close(clip[18] / clip[19], 1);
    await page.reload();
    await page.waitForFunction(() => document.getElementById("stats").textContent.includes("三角形"));
    for (const tilt of ['0', '55', '80']) {
      await page.locator('#tilt').fill(tilt);
      for (const zoom of ['0.5', '4']) {
        await page.locator('#zoom').fill(zoom);
        await page.locator('#bearing').fill('135');
        assert.equal(await page.evaluate(() => gl.getError()), 0);
      }
    }
    await page.locator('#zoom').fill('1');
    await page.locator('#bearing').fill('0');
    await page.locator('#tilt').fill('0');
    await page.screenshot({ path: path.join(path.dirname(fileURLToPath(url)), 'perspective-top.png') });
    await page.locator('#tilt').fill('55');
    await page.screenshot({ path: path.join(path.dirname(fileURLToPath(url)), 'perspective.png') });
    await page.selectOption('#method','shaded');
    await page.locator('#sunHeight').fill('10');
    const shadows=await page.evaluate(()=>{
      function pixels(){const p=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);return p;}
      document.getElementById("castShadow").checked=false;draw();const clear=pixels();
      document.getElementById("castShadow").checked=true;draw();const shadow=pixels();
      let darker=0,brighter=0;
      for(let i=0;i<clear.length;i+=4){if(clear[i]-shadow[i]>3)darker++;if(shadow[i]-clear[i]>3)brighter++;}
      const before=shadowRevision;draw();const cached=shadowRevision===before;
      document.getElementById("light").value="135";draw();
      return {darker,brighter,cached,updated:shadowRevision===before+1,error:gl.getError()};
    });
    assert.ok(shadows.darker>100,'地形が遮る影で画素が暗くなること');
    assert.equal(shadows.brighter,0,'影によって明るくならないこと');
    assert.ok(shadows.cached,'視点の描画で影を再生成しないこと');
    assert.ok(shadows.updated,'光の向きで影を再生成すること');
    assert.equal(shadows.error,0);
    await page.screenshot({path:path.join(path.dirname(fileURLToPath(url)),'cast-shadow.png')});
    assert.deepEqual(errors, []);
    assert.deepEqual(requests, []);
    console.log('透視投影：標高による拡大、中心、前後の描画境界、回転・傾き・拡大、オフライン表示を確認しました。');
  } finally {
    await browser.close();
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
