// LPF比較の各地域・粗段階・比較方式・参照面切替と視点保持を検証する。
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
    await page.goto('http://127.0.0.1:61484/dem-lpf/?region=fuji&quality=50-independent&variant=lpf-balanced&reference=target');
    await page.waitForFunction(() => window.previewReady);
    async function wait(region, error, variant, reference) {
      await page.waitForFunction(({ region, error, variant, reference }) =>
        window.levelInfo?.region === region && window.levelInfo?.targetError === error &&
        window.activeVariant === variant && window.activeReference === reference &&
        document.getElementById('status').textContent.startsWith('元PNGの全'), { region, error, variant, reference });
      assert.equal(await page.evaluate(() => previewError), 0);
    }
    for (const region of ['fuji', 'yari', 'tsukuba']) {
      await page.selectOption('#regionSelect', region);
      await wait(region, 50, 'lpf-balanced', '50-target');
      await page.mouse.move(1000, 700); await page.mouse.down({ button: 'right' });
      await page.mouse.move(1020, 710); await page.mouse.up({ button: 'right' });
      for (const variant of ['original', 'lpf', 'lpf-stable', 'lpf-balanced']) {
        const camera = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
        await page.selectOption('#variantSelect', variant);
        await wait(region, 50, variant, '50-target');
        assert.deepEqual(await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] })), camera);
        await page.screenshot({ path: `build/mls-lpf-dem-v2/${region}-${variant}.png` });
      }
      for (const error of [50, 100, 200, 400, 800, 1600, 3200]) {
        await page.selectOption('#modelSelect', `${error}-independent`);
        await wait(region, error, 'lpf-balanced', `${error}-target`);
        await page.selectOption('#referenceSelect', 'dem');
        await wait(region, error, 'lpf-balanced', 'dem');
        assert.ok(await page.locator('#status').textContent().then(s => s.includes('LPF目標との差')));
        await page.selectOption('#referenceSelect', 'target');
        await wait(region, error, 'lpf-balanced', `${error}-target`);
        results.push(await page.evaluate(() => ({ region: levelInfo.region, error: levelInfo.targetError,
          sigma: levelInfo.sigmaMeters, gl: previewError })));
      }
      await page.selectOption('#modelSelect', '50-independent');
      await wait(region, 50, 'lpf-balanced', '50-target');
    }
    await page.selectOption('#modelSelect', '25-independent');
    await wait('tsukuba', 25, 'original', 'dem');
    await page.selectOption('#variantSelect', 'lpf-balanced');
    await wait('tsukuba', 50, 'lpf-balanced', 'dem');
    await page.selectOption('#range', '32000');
    await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('元PNGの全'));
    assert.equal(await page.evaluate(() => previewError), 0);
    assert.equal(results.length, 21); assert.deepEqual(errors, []);
    fs.writeFileSync('build/mls-lpf-dem-v2/browser.json', JSON.stringify({ results, errors }, null, 2));
    console.log(JSON.stringify({ passed: true, levels: results.length, errors }));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
