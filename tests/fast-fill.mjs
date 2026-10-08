import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';
const reportDir = process.argv[3] || 'docs/verification';
await mkdir(reportDir, { recursive: true });
const require = createRequire(new URL('../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const executablePath = process.argv[2];
if (!executablePath) throw new Error('Pass an existing Chromium executable');
const optimized = await readFile('assets/ocs-rust.user.js', 'utf8');
const baseline = await readFile('assets/ocs-unlimited.user.js', 'utf8');
function library(source) {
  const code = source.slice(source.indexOf('var __defProp'), source.indexOf('\nconst STYLE = `'));
  return code.replace('exports2.start = lib.start;',
    'globalThis.OCS = exports2; globalThis.EUS = lib; globalThis.fastFill = typeof rustFastFill === "undefined" ? null : rustFastFill; exports2.start = lib.start;');
}
const browser = await chromium.launch({ executablePath, headless: true });
const records = [];
async function fixture(source, options = {}) {
  const page = await browser.newPage();
  await page.route('**/*', route => route.abort());
  await page.clock.install({ time: new Date('2026-10-04T00:00:00Z') });
  await page.clock.pauseAt(new Date('2026-10-04T00:00:01Z'));
  await page.setContent('<!doctype html><main id="questions"></main><button id="submit">Submit</button>');
  await page.addScriptTag({ content: library(source) });
  await page.evaluate(options => {
    const { OCS, EUS, fastFill } = window;
    OCS.CommonProject.scripts.settings.cfg = { rustFastFill: options.enabled !== false, rustFillBatchSize: options.batchSize };
    const count = options.count || 12;
    const kinds = ['single', 'multiple', 'judgement', 'completion'];
    window.metrics = { clicks: [], resolved: [], active: 0, maxActive: 0, updates: 0,
      maxUpdates: 0, requests: 0, statusRequests: 0, submits: 0, done: false, elapsed: 0 };
    const m = window.metrics;
    document.querySelector('#submit').onclick = () => m.submits++;
    for (let i = 0; i < count; i++) {
      const kind = options.singleOnly ? 'single' : kinds[i % kinds.length];
      const root = document.createElement('section');
      root.className = options.otherPlatform ? 'other' : 'questionLi';
      root.dataset.index = i; root.dataset.kind = kind;
      root.innerHTML = `<h3>合成题目 ${i}</h3>`;
      const labels = kind === 'judgement' ? ['正确', '错误'] : ['甲', '乙', '丙'];
      if (kind === 'completion') {
        root.insertAdjacentHTML('beforeend', '<textarea class="option"></textarea><textarea class="option"></textarea>');
      } else {
        for (const label of labels) {
          const option = document.createElement('button');
          option.className = 'option'; option.textContent = label;
          option.onclick = () => { option.dataset.checked = 'true'; m.clicks.push([i, label]); };
          root.append(option);
        }
      }
      document.querySelector('#questions').append(root);
    }
    const wrappers = [{ name: 'fixture', url: options.remote ? 'https://example.test/t/fixture/answer' : 'http://127.0.0.1:18766/t/fixture/answer',
      method: 'post', type: 'GM_xmlhttpRequest', contentType: 'json',
      data: { title: '${title}', type: '${type}' },
      handler: 'return r => { if(r.code !== 1) throw new Error("fixture missing answer"); return [r.question, r.answer]; }' }];
    window.GM_xmlhttpRequest = request => {
      let response;
      if (request.url.endsWith('/status')) {
        m.statusRequests++;
        response = { implementation: options.nonRust ? 'python' : 'rust', status: 'running' };
      } else {
        const q = JSON.parse(request.data); const i = Number(q.title.split(' ').at(-1));
        const first = m.requests++ === 0;
        response = { code: i === options.missing ? 0 : 1, question: q.title, batch_size: count,
          cached: !first, answer: q.type === 'multiple' ? '甲#丙' : q.type === 'completion' ? '第一空#第二空' : q.type === 'judgement' ? '正确' : '甲' };
      }
      queueMicrotask(() => request.onload({ status: 200, responseText: JSON.stringify(response) }));
    };
    const start = performance.now();
    const worker = window.worker = new OCS.OCSWorker({
      rustFastFill: true,
      root: Array.from(document.querySelectorAll('section')),
      elements: { title: 'h3', options: '.option' },
      thread: 1, answerSeparators: ['#'],
      answerer: async (elements, ctx) => {
        const title = elements.title[0].textContent;
        if (options.pageCache) return [{ name: 'page cache', results: [{ question: title, answer: '甲', extra_data: { cache: true } }] }];
        if (fastFill) await fastFill.waitBeforeSearch(wrappers, 3);
        else await EUS.$.sleep(3000);
        return OCS.defaultAnswerWrapperHandler(wrappers, { title, type: ctx.type });
      },
      work: {
        type: ctx => ctx.root.dataset.kind,
        handler: async (kind, answer, option, ctx) => {
          m.active++; m.maxActive = Math.max(m.maxActive, m.active);
          if (kind === 'completion') { option.value = answer; m.clicks.push([Number(ctx.root.dataset.index), answer]); }
          else option.click();
          await EUS.$.sleep(500);
          m.active--;
        }
      },
      onResultsUpdate: async (result, index) => {
        m.updates++; m.maxUpdates = Math.max(m.maxUpdates, m.updates);
        await EUS.$.sleep(1);
        if (result.resolved) m.resolved.push([index, result.result.finish]);
        m.updates--;
      }
    });
    worker.doWork().then(() => { m.done = true; m.elapsed = performance.now() - start; })
      .catch(error => { m.error = String(error); });
  }, options);
  return page;
}
async function run(name, source, options = {}) {
  const page = await fixture(source, options);
  try {
    await page.clock.runFor((options.count || 12) * 4300 + 10000);
    const m = await page.evaluate(() => window.metrics);
    assert(!m.error, m.error); assert(m.done, name + ' must finish');
    assert.equal(m.resolved.length, options.count || 12);
    assert.equal(m.submits, 0, 'must not submit');
    assert.equal(m.resolved.filter(x => !x[1]).length, options.missing === undefined ? 0 : 1);
    if (source === optimized) assert.equal(m.maxUpdates, 1, 'result storage must remain serialized');
    const controls = await page.evaluate(() => Array.from(document.querySelectorAll('section')).map(root => ({
      kind: root.dataset.kind, index: Number(root.dataset.index),
      chosen: Array.from(root.querySelectorAll('[data-checked]')).map(el => el.textContent),
      values: Array.from(root.querySelectorAll('textarea')).map(el => el.value)
    })));
    for (const row of controls) {
      const expected = row.index === options.missing ? [] : row.kind === 'multiple' ? ['甲', '丙'] : row.kind === 'judgement' ? ['正确'] : row.kind === 'completion' ? [] : ['甲'];
      assert.deepEqual(row.chosen, expected, name + ' question ' + row.index);
      if (row.kind === 'completion') assert.deepEqual(row.values, row.index === options.missing ? ['', ''] : ['第一空', '第二空']);
    }
    const record = { name, virtualMs: m.elapsed, maxConcurrentFill: m.maxActive, requests: m.requests, statusRequests: m.statusRequests, questions: controls.length, filled: m.resolved.filter(x => x[1]).length, missing: options.missing === undefined ? 0 : 1, submits: m.submits };
    records.push(record); console.log(JSON.stringify(record)); return record;
  } finally { await page.close(); }
}
try {
  const old = await run('baseline-85', baseline, { count: 85, singleOnly: true });
  const fast = await run('optimized-85', optimized, { count: 85, singleOnly: true });
  assert(fast.virtualMs < old.virtualMs / 5); assert.equal(fast.maxConcurrentFill, 16);
  const max = await run('batch-32-200-questions', optimized, { count: 200, singleOnly: true, batchSize: 32 }); assert.equal(max.maxConcurrentFill, 32);
  const capped = await run('batch-size-cap', optimized, { count: 85, singleOnly: true, batchSize: 999 }); assert.equal(capped.maxConcurrentFill, 32);
  await run('mixed-types', optimized);
  await run('missing-answer', optimized, { missing: 4 });
  const off = await run('disabled', optimized, { enabled: false }); assert.equal(off.maxConcurrentFill, 1); assert.equal(off.statusRequests, 0);
  const remote = await run('remote-provider', optimized, { remote: true }); assert.equal(remote.maxConcurrentFill, 1); assert.equal(remote.statusRequests, 0);
  const nonRust = await run('non-rust-local-provider', optimized, { nonRust: true }); assert.equal(nonRust.maxConcurrentFill, 1);
  const other = await run('other-platform', optimized, { otherPlatform: true }); assert.equal(other.maxConcurrentFill, 1);
  const cached = await run('page-cache', optimized, { pageCache: true, singleOnly: true }); assert(cached.maxConcurrentFill > 1 && cached.maxConcurrentFill <= 16);
  for (const close of [false, true]) {
    const page = await fixture(optimized, { singleOnly: true, pageCache: true });
    await page.clock.runFor(100);
    await page.evaluate(() => window.worker.emit('stop'));
    await page.clock.runFor(2000);
    const before = await page.evaluate(() => window.metrics.clicks.length);
    await page.clock.runFor(2000);
    assert.equal(await page.evaluate(() => window.metrics.clicks.length), before, 'pause prevents next batch');
    await page.evaluate(close => window.worker.emit(close ? 'close' : 'continuate'), close);
    await page.clock.runFor(20000);
    const after = await page.evaluate(() => window.metrics);
    assert(after.done);
    if (close) assert.equal(after.clicks.length, before, 'close must not apply any more answers');
    else assert.equal(after.resolved.length, 12);
    records.push({ name: close ? 'close-while-paused' : 'pause-resume', passed: true });
    await page.close();
  }
  await writeFile(join(reportDir, 'fast-fill.json'), JSON.stringify({ realChromium: true, clock: 'virtual', realModelCalls: 0, liveCourseTested: false, records }, null, 2) + '\n');
} finally { await browser.close(); }
