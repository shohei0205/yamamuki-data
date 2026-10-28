// 新旧の補間半径・参照面・全粗段階の描画と視点保持を確認する。
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true,
    args: ['--ignore-gpu-blocklist'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1800, height: 1100 } });
    const errors = []; const results = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.goto('http://127.0.0.1:61484/dem-lpf/?region=fuji&quality=50-independent&variant=lpf-smooth&reference=target');
    await page.waitForFunction(() => window.previewReady);
    async function wait(region, error, variant) {
      await page.waitForFunction(({ region, error, variant }) => levelInfo.region === region &&
        levelInfo.targetError === error && activeVariant === variant &&
        document.getElementById('status').textContent.startsWith('元PNGの全'), { region, error, variant });
      assert.equal(await page.evaluate(() => previewError), 0);
    }
    for (const region of ['fuji', 'yari', 'tsukuba']) {
      await page.selectOption('#regionSelect', region); await wait(region, 50, 'lpf-smooth');
      const camera = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
      for (const variant of ['lpf-balanced', 'lpf-smooth']) {
        await page.selectOption('#variantSelect', variant); await wait(region, 50, variant);
        assert.deepEqual(await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] })), camera);
        await page.screenshot({ path: `build/mls-smooth-radius-v1/${region}-${variant}.png` });
      }
      for (const error of [50, 100, 200, 400, 800, 1600, 3200]) {
        await page.selectOption('#modelSelect', `${error}-independent`); await wait(region, error, 'lpf-smooth');
        assert.equal(await page.evaluate(() => levelInfo.supportMode), 'smooth-log-radius-cubic-v1');
        await page.selectOption('#referenceSelect', 'dem');
        await page.waitForFunction(() => activeReference === 'dem' && document.getElementById('status').textContent.startsWith('元PNGの全'));
        await page.selectOption('#referenceSelect', 'target');
        await page.waitForFunction(error => activeReference === error+'-target' && document.getElementById('status').textContent.startsWith('元PNGの全'), error);
        results.push(await page.evaluate(() => ({ region: levelInfo.region, error: levelInfo.targetError, gl: previewError })));
      }
      await page.selectOption('#modelSelect', '50-independent'); await wait(region, 50, 'lpf-smooth');
    }
    assert.equal(results.length, 21); assert.deepEqual(errors, []);
    fs.writeFileSync('build/mls-smooth-radius-v1/browser.json', JSON.stringify({ results, errors }, null, 2));
    console.log(JSON.stringify({ passed: true, levels: results.length, errors }));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
