// 地域選択サーバーを起動した状態で、ポリゴン表示と影を検証する。
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

async function main() {
  const browser = await chromium.launch({ headless: true, channel: 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(process.argv[2] || 'http://127.0.0.1:61434/');
    const reports = [];
    for (const preset of ['35.36,138.73', '36.00,138.37', '35.37,133.55', 'nationwide']) {
      await page.selectOption('#preset', preset);
      await page.waitForFunction(() => document.querySelector('#status').textContent.startsWith('適応ポリゴン'));
      const frame = await page.locator('#terrain').contentFrame();
      const report = await frame.locator('body').evaluate(() => {
        const report = { zoom: data.coverage.zoom, vertices: data.mesh.length,
          triangles: data.indices.length / 3, polygon: data.polygon };
        if (!data.mesh.every(v => v.every(Number.isFinite))) throw Error('頂点に不正な値があります');
        if (!data.indices.every(i => Number.isInteger(i) && i >= 0 && i < data.mesh.length)) throw Error('接続が不正です');
        document.querySelector('#method').value = 'wireframe';
        rebuildWire(); draw();
        report.wireIndices = wireCount;
        document.querySelector('#method').value = 'shaded';
        document.querySelector('#sunHeight').value = '15';
        rebuildWire(); draw();
        report.glError = gl.getError();
        report.spacingDisabled = document.querySelector('#spacing').disabled;
        return report;
      });
      assert.equal(report.polygon, true);
      assert.equal(report.glError, 0);
      assert.equal(report.spacingDisabled, true);
      assert.ok(report.wireIndices > 0);
      assert.ok(report.vertices > 0 && report.triangles > 0);
      assert.equal(report.zoom, preset === 'nationwide' ? 4 : 14);
      reports.push({ preset, ...report });
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify(reports, null, 2));
  } finally {
    await browser.close();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
