import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';

const source = (await readFile(process.argv[3] || 'assets/ocs-rust.user.js', 'utf8')).replace(/\r\n/g, '\n');
const require = createRequire(new URL('../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const library = source.slice(source.indexOf('var __defProp'), source.indexOf('\nconst STYLE = `'))
  .replace('exports2.start = lib.start;', 'globalThis.OCS = exports2; globalThis.EUS = lib; globalThis.progress = rustWorkProgress; exports2.start = lib.start;');
const browser = await chromium.launch({ executablePath: process.argv[2], headless: true });
const records = [];

async function check(name, run) {
  const page = await browser.newPage();
  try {
    await page.route('**/*', route => route.abort());
    await page.clock.install({ time: new Date('2026-10-08T00:00:00Z') });
    await page.clock.pauseAt(new Date('2026-10-08T00:00:01Z'));
    await page.setContent('<main id="questions"><section><h3>Synthetic</h3><button>Alpha</button></section></main><div id="panel"></div>');
    await page.addScriptTag({ content: library });
    await page.evaluate(() => {
      const { OCS, EUS, progress } = window;
      window.ui = OCS.CommonProject.scripts.workResults;
      window.resultKey = 'common.work-results.results';
      OCS.CommonProject.scripts.settings.cfg = { rustFastFill: true, rustFillBatchSize: 16 };
      ui.cfg = { type: 'numbers', currentResultIndex: 0, totalQuestionCount: 1, requestedCount: 1, resolvedCount: 1 };
      window.sample = finish => [{ question: 'Synthetic', requested: true, resolved: finish, finish,
        searchInfos: [{ name: 'synthetic AI', results: [{ question: 'Synthetic', answer: 'Alpha' }] }] }];
      window.progressValue = filled => ({ phase: 'done', total: 2, filled, requested: 2, resolved: 2, elapsed: 1 });
      window.mountPanel = () => document.getElementById('panel').append(ui.methods.createWorkResultsPanel());
      window.startWorker = (opts = {}) => {
        window.metrics = { done: false, updates: 0, filled: 0 };
        window.worker = new OCS.OCSWorker({ rustFastFill: true,
          root: Array.from(document.querySelectorAll('section')), elements: { title: 'h3', options: 'button' },
          answerer: async () => [{ name: 'synthetic AI', results: [{ answer: 'Alpha' }] }],
          work: async () => { metrics.filled++; return { finish: true }; }, ...opts });
        worker.doWork().then(() => { metrics.done = true; }).catch(error => { metrics.error = error.message; });
      };
    });
    await run(page);
    records.push({ name, passed: true });
  } catch (error) {
    records.push({ name, passed: false, error: error.message });
    process.exitCode = 1;
  } finally { await page.close(); }
}

try {
  await check('initial-progress-read-cannot-replace-a-newer-notification', async page => {
    await page.evaluate(() => {
      const read = EUS.$store.getTab.bind(EUS.$store);
      let held = false;
      EUS.$store.getTab = key => {
        if (key !== progress.key || held) return read(key);
        held = true;
        return new Promise(resolve => { window.releaseProgress = () => resolve(progressValue(0)); });
      };
      mountPanel();
    });
    await page.evaluate(() => EUS.$store.setTab(progress.key, progressValue(2)));
    await page.clock.runFor(10);
    await page.evaluate(() => releaseProgress());
    await page.clock.runFor(200);
    assert.match(await page.locator('.rust-work-progress').textContent(), /已填入 2\/2/);
  });

  for (const cleared of [false, true]) {
    await check(cleared ? 'cleared-results-cannot-be-restored-by-a-late-read' : 'late-results-read-cannot-undo-finished-numbers', async page => {
      await page.evaluate(() => {
        window.reads = [];
        ui.methods.getResults = () => new Promise(resolve => reads.push(resolve));
        mountPanel();
      });
      await page.clock.runFor(110);
      await page.evaluate(() => EUS.$store.setTab(resultKey, sample(true)));
      await page.clock.runFor(110);
      assert.equal(await page.evaluate(() => reads.length), 2);
      await page.evaluate(cleared => reads[1](cleared ? [] : sample(true)), cleared);
      await page.evaluate(() => reads[0](sample(false)));
      assert.equal(await page.locator('.search-infos-num').count(), cleared ? 0 : 1);
      if (!cleared) assert.equal(await page.locator('.search-infos-num.finish').count(), 1);
    });
  }

  await check('panel-reads-current-state-after-asynchronous-subscription', async page => {
    await page.evaluate(async () => {
      await ui.methods.setResults(sample(false));
      const subscribe = EUS.$store.addTabChangeListener.bind(EUS.$store);
      EUS.$store.addTabChangeListener = (key, listener) => key === resultKey ? new Promise(resolve => {
        window.releaseSubscription = () => resolve(subscribe(key, listener));
      }) : subscribe(key, listener);
      mountPanel();
    });
    await page.clock.runFor(110);
    await page.evaluate(async () => { await ui.methods.setResults(sample(true)); releaseSubscription(); });
    await page.clock.runFor(110);
    assert.equal(await page.locator('.search-infos-num.finish').count(), 1);
  });

  for (const failure of [false, true]) {
    await check(failure ? 'final-save-failure-does-not-report-success' : 'worker-completion-waits-for-final-state-persistence', async page => {
      await page.evaluate(() => {
        const save = EUS.$store.setTab.bind(EUS.$store);
        EUS.$store.setTab = (key, value) => key === progress.key && value?.phase === 'done' ? new Promise((resolve, reject) => {
          window.releaseFinalSave = fail => fail ? reject(new Error('synthetic final persistence failure')) : resolve(save(key, value));
        }) : save(key, value);
        startWorker();
      });
      await page.clock.runFor(500);
      assert.equal(await page.evaluate(() => typeof releaseFinalSave), 'function');
      const prematurelyDone = await page.evaluate(() => metrics.done);
      await page.evaluate(failure => releaseFinalSave(failure), failure);
      await page.clock.runFor(10);
      const state = await page.evaluate(() => ({ ...metrics, running: worker.isRunning }));
      assert.equal(prematurelyDone, false, 'doWork must not resolve before its final save');
      assert.equal(state.running, false);
      assert.equal(state.done, !failure);
      if (failure) assert.match(state.error, /synthetic final persistence failure/);
    });
  }

  await check('closed-worker-skips-already-queued-result-callbacks', async page => {
    await page.evaluate(() => {
      const root = document.querySelector('section');
      for (let i = 1; i < 8; i++) root.parentElement.append(root.cloneNode(true));
      startWorker({ thread: 8, onResultsUpdate() {
        metrics.updates++;
        if (metrics.updates === 2) return new Promise(resolve => { window.releaseUpdate = resolve; });
      } });
    });
    await page.clock.runFor(10);
    assert.equal(await page.evaluate(() => metrics.updates), 2);
    await page.evaluate(() => { worker.emit('close'); releaseUpdate(); });
    await page.clock.runFor(500);
    const state = await page.evaluate(() => metrics);
    assert.equal(state.updates, 2, 'callbacks queued by a closed task must not overwrite a replacement task');
    assert.equal(state.done, true);
  });
  console.log(JSON.stringify({ realChromium: true, realModelCalls: 0, records }, null, 2));
} finally { await browser.close(); }
