// 6点前後の試験を3地域で表示し、広い版との切替・視点保持を確認する。
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
    await page.goto('http://127.0.0.1:61484/dem-xy/?variant=six-xy');
    await page.waitForFunction(() => window.previewReady);
    async function wait(region, variant) {
      await page.waitForFunction(({ region, variant }) => levelInfo.region === region && activeVariant === variant &&
        document.getElementById('status').textContent.startsWith('元PNGの全'), { region, variant });
      assert.equal(await page.evaluate(() => previewError), 0);
    }
    for (const region of ['fuji', 'yari', 'tsukuba']) {
      await page.selectOption('#variantSelect', 'six-xy');
      await page.selectOption('#regionSelect', region); await wait(region, 'six-xy');
      const camera = await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] }));
      for (const variant of ['six-trained', 'six-xy', 'six-more']) {
        await page.selectOption('#variantSelect', variant); await wait(region, variant);
        assert.deepEqual(await page.evaluate(() => ({ yaw, tilt, span, pan: [...pan] })), camera);
        if (variant === 'six-trained') {
          assert.equal(await page.evaluate(() => levelInfo.neighborCountMin), 6);
          assert.equal(await page.evaluate(() => levelInfo.neighborCountMedian), 8);
          assert.equal(await page.evaluate(() => levelInfo.additionalDisplaySmoothing), false);
        }
        await page.screenshot({ path: `build/mls-xy-placement-v1/${region}-${variant}.png` });
        results.push(await page.evaluate(() => ({ region: levelInfo.region, variant: activeVariant, gl: previewError })));
      }
      await page.selectOption('#referenceSelect', 'dem');
      await page.waitForFunction(() => activeReference === 'dem' && document.getElementById('status').textContent.startsWith('元PNGの全'));
      await page.selectOption('#referenceSelect', 'target');
      await page.waitForFunction(() => activeReference === '50-target' && document.getElementById('status').textContent.startsWith('元PNGの全'));
    }
    assert.equal(results.length, 9); assert.deepEqual(errors, []);
    fs.writeFileSync('build/mls-xy-placement-v1/browser.json', JSON.stringify({ results, errors }, null, 2));
    console.log(JSON.stringify({ passed: true, cases: results.length, errors }));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
