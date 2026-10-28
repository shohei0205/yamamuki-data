// 元DEM比較の3方式・範囲切替・視点保持を確認する。
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true,
    args: ['--ignore-gpu-blocklist'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1800, height: 1100 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('http://127.0.0.1:61484/yari-dem/?quality=25-independent');
    await page.waitForFunction(() => window.previewReady);
    await page.mouse.move(1000, 700);
    await page.mouse.down({ button: 'right' });
    await page.mouse.move(1040, 720);
    await page.mouse.up({ button: 'right' });
    const results = [];
    for (const kind of ['25-independent', '12.5-independent', '12.5-residual']) {
      const before = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
      await page.selectOption('#modelSelect', kind);
      await page.waitForFunction(kind => location.search === '?quality=' + kind &&
        document.getElementById('status').textContent.startsWith('元PNGの全'), kind);
      const after = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
      assert.deepEqual(after, before);
      const result = await page.evaluate(() => ({ info: window.levelInfo,
        error: window.previewError, label: document.querySelector('strong').textContent }));
      assert.equal(result.error, 0);
      assert.equal(result.info.reference, 'original-PNG-DEM');
      assert.equal(result.info.samples, 11222500);
      if (kind === '12.5-residual') assert.ok(result.label.includes('親込み'));
      await page.screenshot({ path: `build/mls-dem-levels-yari-v1/${kind}.png` });
      results.push({ kind, ...result });
    }
    await page.selectOption('#range', '32000');
    await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('元PNGの全'));
    assert.equal(await page.evaluate(() => window.previewError), 0);
    assert.deepEqual(errors, []);
    fs.writeFileSync('build/mls-dem-levels-yari-v1/browser.json', JSON.stringify({ results, errors }, null, 2));
    console.log(JSON.stringify({ passed: true, variants: results.length, errors }));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
