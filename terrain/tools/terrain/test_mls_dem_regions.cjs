// 3地域×全段階を描画し、地域・品質表示、視点保持、WebGLを確認する。
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true,
    args: ['--ignore-gpu-blocklist'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1800, height: 1100 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.goto('http://127.0.0.1:61484/dem-regions/?region=fuji&quality=50-independent');
    await page.waitForFunction(() => window.previewReady);
    const results = [];
    for (const region of ['fuji', 'yari', 'tsukuba']) {
      await page.selectOption('#regionSelect', region);
      await page.waitForFunction(region => window.levelInfo?.region === region &&
        document.getElementById('status').textContent.startsWith('元PNGの全'), region);
      const kinds = await page.locator('#modelSelect option').evaluateAll(options => options.map(o => o.value));
      for (const kind of kinds) {
        const before = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
        await page.selectOption('#modelSelect', kind);
        await page.waitForFunction(({region, kind}) => location.search === '?region='+region+'&quality='+kind &&
          document.getElementById('status').textContent.startsWith('元PNGの全'), { region, kind });
        assert.deepEqual(await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] })), before);
        const result = await page.evaluate(() => ({ info: window.levelInfo, error: window.previewError }));
        assert.equal(result.error, 0);
        assert.equal(result.info.region, region);
        assert.equal(result.info.reference, 'original-PNG-DEM');
        assert.equal(String(result.info.targetError), kind.split('-')[0]);
        results.push({ region, kind, ...result });
        if (['50-independent', '12.5-residual'].includes(kind))
          await page.screenshot({ path: `build/mls-dem-regions-v1/${region}-${kind}.png` });
      }
    }
    assert.equal(results.length, 30);
    assert.deepEqual(errors, []);
    fs.writeFileSync('build/mls-dem-regions-v1/browser.json', JSON.stringify({ results, errors }, null, 2));
    console.log(JSON.stringify({ passed: true, variants: results.length, errors }));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
