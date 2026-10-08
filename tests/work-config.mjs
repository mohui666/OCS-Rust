import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { EventEmitter } from 'node:events';
import vm from 'node:vm';

const source = (await readFile(process.argv[3] || 'assets/ocs-rust.user.js', 'utf8')).replace(/\r\n/g, '\n');
const controls = source.slice(source.indexOf('  let globalControlPanel = null;'), source.indexOf('  function optimizationElementWithImage('));
const records = [];
const tick = () => new Promise(resolve => setImmediate(resolve));
async function settle() { for (let i = 0; i < 8; i++) await tick(); }
function fixture(desktop = false) {
  const buttons = [], errors = [], workers = [];
  const cfg = { answererWrappers: [], disabledAnswererWrapperNames: [] };
  const remote = { wrappers: [], fail: false, refreshes: 0, wait: null };
  const node = () => ({ style: {}, remove() {}, replaceChildren() {} });
  const script = Object.assign(new EventEmitter(), { panel: { body: node() } });
  const context = vm.createContext({
    CommonProject: { scripts: {
      render: { methods: { pin() {} } },
      settings: { methods: { getWorkOptions: () => structuredClone({ ...cfg,
        answererWrappers: cfg.answererWrappers.filter(item => !cfg.disabledAnswererWrapperNames.includes(item.name)) }) } },
      workResults: { methods: { createWorkResultsPanel: node } }
    } },
    lib: { h: node, MessageElement: class {}, $ui: { button(value) {
      const button = { ...node(), value }; buttons.push(button); return button;
    } }, $message: { error({ content }) { errors.push(content); }, warn: node } },
    workPreCheckMessage: () => node(), closeAnswerWrapperEmptyWarning: async () => {},
    hasRustDesktop: () => desktop,
    refreshManagedAI: async () => {
      remote.refreshes++;
      if (remote.wait) await remote.wait;
      if (remote.fail) throw new Error('synthetic settings connection failure');
      cfg.answererWrappers = structuredClone(remote.wrappers);
    },
    script, options: { enable_control_panel: false, workerProvider(options) {
      const worker = Object.assign(new EventEmitter(), { options });
      worker.on('close', () => { worker.closed = true; });
      workers.push(worker); return worker;
    } }
  });
  vm.runInContext(controls + '\ncommonWork(script, options);', context);
  script.emit('render');
  // Render the same controls without needing a browser UI for these state tests.
  context.options.enable_control_panel = true;
  context.CommonProject.scripts.workResults.on = () => {};
  script.emit('render');
  const click = label => buttons.findLast(button => button.value.includes(label)).onclick();
  return { cfg, remote, workers, errors, click };
}
const ai = { name: 'AI current', model: 'unchanged-model' };

const enabledLater = fixture();
enabledLater.cfg.answererWrappers = [ai];
enabledLater.click('开始答题'); await settle();
assert.deepEqual(enabledLater.workers[0]?.options.answererWrappers, [ai], 'start must not keep the initial empty source list');
records.push('enable-after-page-load');

enabledLater.cfg.answererWrappers = [ai, { name: 'AI replacement' }];
enabledLater.cfg.disabledAnswererWrapperNames = [ai.name];
enabledLater.click('重新答题'); await settle();
assert.equal(enabledLater.workers[0].closed, true);
assert.deepEqual(enabledLater.workers[1]?.options.answererWrappers, [{ name: 'AI replacement' }]);
records.push('restart-reloads-enabled-sources');

const synced = fixture(true);
let release;
synced.remote.wrappers = [ai];
synced.remote.wait = new Promise(resolve => { release = resolve; });
synced.click('开始答题'); synced.click('开始答题'); await settle();
assert.equal(synced.workers.length, 0);
release(); await settle();
assert.equal(synced.remote.refreshes, 1);
assert.equal(synced.workers.length, 1);
assert.deepEqual(synced.workers[0].options.answererWrappers, [ai]);
records.push('desktop-sync-before-start-and-double-click-guard');

const failed = fixture(true);
failed.remote.fail = true;
failed.click('开始答题'); await settle();
assert.equal(failed.workers.length, 0);
assert.match(failed.errors[0], /synthetic settings connection failure/);
failed.remote.fail = false; failed.remote.wrappers = [ai];
failed.click('开始答题'); await settle();
assert.deepEqual(failed.workers[0]?.options.answererWrappers, [ai]);
records.push('sync-failure-is-visible-and-retryable');

const empty = fixture(true);
empty.click('开始答题'); await settle();
assert.equal(empty.workers.length, 0);
assert.equal(empty.errors.length, 1);
records.push('no-task-starts-without-enabled-sources');

const require = createRequire(new URL('../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const library = source.slice(source.indexOf('var __defProp'), source.indexOf('\nconst STYLE = `'))
  .replace('exports2.start = lib.start;', 'globalThis.OCS = exports2; exports2.start = lib.start;');
const browser = await chromium.launch({ executablePath: process.argv[2], headless: true });
try {
  const page = await browser.newPage();
  await page.route('**/*', route => route.abort());
  await page.setContent('<div id="questions"></div>');
  await page.addScriptTag({ content: library });
  const result = await page.evaluate(async () => {
    const { OCS } = window;
    OCS.CommonProject.scripts.settings.cfg = { rustFillBatchSize: 16, disabledAnswererWrapperNames: [] };
    const parent = document.getElementById('questions');
    for (let i = 0; i < 85; i++) {
      const root = document.createElement('section');
      root.innerHTML = `<h3>Synthetic ${i}</h3><button class="option">2</button>`;
      parent.append(root);
    }
    let requests = 0, filled = 0;
    window.GM_xmlhttpRequest = request => {
      requests++;
      queueMicrotask(() => request.onload({ status: 200, responseText: JSON.stringify({ answer: '2' }) }));
      return { abort() {} };
    };
    const wrapper = { name: 'synthetic', url: 'https://example.test/answer', method: 'post', type: 'GM_xmlhttpRequest',
      data: { title: '${title}' }, handler: 'return r => ["synthetic", r.answer];' };
    const opts = { rustFastFill: true, root: Array.from(parent.children), elements: { title: 'h3', options: '.option' },
      answerer: (elements, ctx) => OCS.defaultAnswerWrapperHandler([wrapper], { title: elements.title[0].textContent, rustProgress: ctx.rustProgress }),
      work: { type: 'single', handler: async () => { filled++; } } };
    const results = await new OCS.OCSWorker(opts).doWork();
    const errors = await new OCS.OCSWorker({ ...opts, root: [parent.firstElementChild],
      answerer: async () => { throw new Error('original AI connection error'); } }).doWork();
    const missing = await new OCS.OCSWorker({ ...opts, root: [parent.firstElementChild], answerer: async () => [] }).doWork();
    return { requests, filled, finished: results.filter(result => result.result.finish).length,
      error: errors[0].error, missing: missing[0].error, errorFinished: errors[0].result.finish };
  });
  assert.equal(result.requests, 85);
  assert.equal(result.finished, 85);
  assert.equal(result.filled, 85);
  assert.match(result.error, /original AI connection error/);
  assert.equal(result.errorFinished, false);
  assert.match(result.missing, /搜索不到答案/);
  records.push('85-synthetic-questions-request-and-fill', 'original-request-error-preserved', 'empty-result-fallback-retained');
  console.log(JSON.stringify({ realChromium: true, realModelCalls: 0, liveCourseTested: false, passed: records }, null, 2));
} finally { await browser.close(); }
