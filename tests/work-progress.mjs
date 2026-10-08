import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';
const reportDir = process.argv[3] || 'docs/verification';
await mkdir(reportDir, { recursive: true });
const require = createRequire(new URL('../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const source = await readFile('assets/ocs-rust.user.js', 'utf8');
const library = source.slice(source.indexOf('var __defProp'), source.indexOf('\nconst STYLE = `'))
  .replace('exports2.start = lib.start;', 'globalThis.OCS = exports2; globalThis.EUS = lib; globalThis.progress = rustWorkProgress; exports2.start = lib.start;');
const browser = await chromium.launch({ executablePath: process.argv[2], headless: true });
const records = [];
async function fixture(options = {}) {
  const page = await browser.newPage({ viewport: { width: 800, height: 500 } });
  await page.route('**/*', route => route.abort());
  await page.clock.install({ time: new Date('2026-10-04T00:00:00Z') });
  await page.clock.pauseAt(new Date('2026-10-04T00:00:01Z'));
  await page.setContent('<style>body{font-family:system-ui;padding:30px} #questions{display:none}</style><h3>OCS 运行进度（合成页面）</h3><div id="questions"></div>');
  await page.addScriptTag({ content: library });
  await page.evaluate(options => {
    const { OCS, EUS, progress } = window;
    const ui = OCS.CommonProject.scripts.workResults;
    OCS.CommonProject.scripts.settings.cfg = { rustFastFill: true, rustFillBatchSize: 16 };
    ui.cfg = { type: 'numbers', currentResultIndex: 0, totalQuestionCount: 0, requestedCount: 0, resolvedCount: 0 };
    ui.methods.init();
    document.body.append(ui.methods.createWorkResultsPanel());
    const count = options.count || 85;
    const m = window.metrics = { polls: 0, requests: 0, writes: 0, filled: 0, done: false, failStatus: !!options.failStatus };
    EUS.$store.addTabChangeListener(progress.key, value => { m.writes++; m.value = value; });
    for (let i = 0; i < count; i++) {
      const root = document.createElement('section'); root.className = 'questionLi';
      root.innerHTML = `<h3>合成题目 ${i}</h3><button class="option">甲</button>`;
      document.querySelector('#questions').append(root);
    }
    const wrapper = { name: 'test', url: options.remote ? 'https://example.test/t/test/answer' : 'http://127.0.0.1:18766/t/test/answer',
      method: 'post', type: 'GM_xmlhttpRequest', data: { title: '${title}', pageQuestions: { handler: 'return () => Array.from(document.querySelectorAll("section h3")).map(e => ({title:e.textContent}));' } },
      handler: 'return r => { if(r.code !== 1) throw new Error("合成查询失败"); return [r.question, r.answer]; }' };
    window.GM_xmlhttpRequest = request => {
      if (request.url.endsWith('/status')) {
        m.polls++;
        queueMicrotask(() => m.failStatus ? request.onerror({}) : request.onload({ status: 200,
          responseText: JSON.stringify({ implementation: options.nonRust ? 'python' : 'rust', status: 'running', inflight: count }) }));
      } else {
        const first = m.requests++ === 0;
        const respond = () => request.onload({ status: 200, responseText: JSON.stringify({ code: options.failure ? 0 : 1,
          question: JSON.parse(request.data).title, answer: '甲', cached: !first, batch_size: count }) });
        if (first) window.release = respond;
        else queueMicrotask(respond);
      }
      return { abort() {} };
    };
    const worker = window.worker = new OCS.OCSWorker({ rustFastFill: true,
      root: Array.from(document.querySelectorAll('section')), elements: { title: 'h3', options: '.option' },
      answerer: (elements, ctx) => OCS.defaultAnswerWrapperHandler([wrapper], { title: elements.title[0].textContent, rustProgress: ctx.rustProgress }),
      work: { type: 'single', handler: async () => { await EUS.$.sleep(500); m.filled++; } },
      onResultsUpdate: (_, __, results) => ui.methods.updateWorkStateByResults(results)
    });
    worker.doWork().then(() => { m.done = true; }).catch(e => { m.error = String(e); });
  }, options);
  await page.clock.runFor(3100);
  return page;
}
const read = page => page.evaluate(() => ({ ...window.metrics, text: document.querySelector('.rust-work-progress')?.textContent }));
try {
  const page = await fixture();
  let m = await read(page);
  assert.equal(m.value.total, 85); assert.equal(m.value.filled, 0); assert.equal(m.value.requested, 0);
  assert.match(m.text, /Thinking/); assert.match(m.text, /当前请求 85 题/); assert.match(m.text, /已返回 0\/85/);
  await page.screenshot({ path: join(reportDir, 'work-progress-thinking.png') });
  await page.clock.fastForward(610000);
  m = await read(page); assert(m.value.elapsed >= 610); assert(!m.done); assert.equal(m.requests, 1);
  await page.evaluate(() => window.worker.emit('stop'));
  await page.clock.runFor(1000); assert.match((await read(page)).text, /已暂停填答/);
  await page.evaluate(() => { window.worker.emit('continuate'); window.release(); });
  await page.clock.runFor(10000);
  m = await read(page); assert(m.done && !m.error); assert.equal(m.filled, 85); assert.match(m.text, /填答完成/);
  assert.equal(m.value.requested, 85); assert.equal(m.value.filled, 85);
  const stopped = [m.polls, m.writes];
  await page.clock.runFor(10000); m = await read(page); assert.deepEqual([m.polls, m.writes], stopped);
  records.push({ name: '85-question-wait-over-610-seconds-pause-resume-fill-and-timer-cleanup', passed: true });
  await page.close();

  const failed = await fixture({ failure: true, count: 3, failStatus: true });
  assert.match((await read(failed)).text, /状态连接失败/);
  await failed.evaluate(() => { window.metrics.failStatus = false; });
  await failed.clock.runFor(3000); assert.match((await read(failed)).text, /Thinking/);
  await failed.evaluate(() => window.release());
  await failed.clock.runFor(3000); m = await read(failed);
  assert(m.done); assert.equal(m.filled, 0); assert.match(m.text, /3 题未填入/);
  records.push({ name: 'status-recovery-and-failed-answers-not-counted-as-filled', passed: true });
  await failed.close();

  const closed = await fixture({ count: 3 });
  await closed.evaluate(() => window.worker.emit('close'));
  await closed.clock.runFor(1000); m = await read(closed); assert.match(m.text, /已停止/);
  const polls = m.polls;
  await closed.evaluate(() => window.release());
  await closed.clock.runFor(10000); m = await read(closed);
  assert(m.done); assert.equal(m.filled, 0); assert.equal(m.polls, polls);
  records.push({ name: 'close-cleans-polls-and-late-answer-does-not-fill', passed: true });
  await closed.close();

  for (const options of [{ remote: true }, { nonRust: true }]) {
    const other = await fixture({ ...options, count: 2 }); m = await read(other);
    assert.doesNotMatch(m.text, /Thinking/);
    if (options.remote) assert.equal(m.polls, 0);
    await other.evaluate(() => { window.worker.emit('close'); window.release(); });
    await other.clock.runFor(1000);
    records.push({ name: options.remote ? 'remote-provider-not-polled' : 'non-rust-provider-not-labelled-thinking', passed: true });
    await other.close();
  }
  await writeFile(join(reportDir, 'work-progress.json'), JSON.stringify({ realChromium: true, clock: 'virtual', realModelCalls: 0, liveCourseTested: false, records }, null, 2) + '\n');
  console.log(JSON.stringify(records, null, 2));
} finally { await browser.close(); }
