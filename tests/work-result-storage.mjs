import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const source = (await readFile(process.argv[3] || 'assets/ocs-rust.user.js', 'utf8')).replace(/\r\n/g, '\n');
const provider = source.slice(source.indexOf('  class GMStoreProvider {'), source.indexOf('  store_provider.GMStoreProvider = GMStoreProvider;'));
const changes = new Map();
let tab = { uid: 'fixture-tab', unrelated: 'preserved', results: { filled: 0 } };
let reads = 0, active = 0, maxActive = 0, failSave = false;
const context = vm.createContext({
  self: {}, top: {}, const_1$1: { $const: { TAB_UID: 'uid' } },
  GM_setValue: (key, value) => changes.set(key, value),
  GM_getTab(callback) {
    const snapshot = structuredClone(tab);
    active++; maxActive = Math.max(maxActive, active);
    // Deliver an older read after a newer one, as asynchronous extension messages can do.
    setTimeout(() => { active--; callback(snapshot); }, reads++ === 0 ? 20 : 1);
  },
  GM_saveTab(value) {
    if (failSave) { failSave = false; throw new Error('synthetic persistence failure'); }
    tab = structuredClone(value);
  }
});
const store = vm.runInContext(provider + '\nnew GMStoreProvider()', context);
await Promise.all([store.setTab('progress', { filled: 0 }), store.setTab('results', { filled: 84 }), store.setTab('progress', { filled: 84 })]);
assert.equal(tab.results.filled, 84, 'an older progress snapshot must not roll back finished answers');
assert.equal(tab.progress.filled, 84);
assert.equal(tab.unrelated, 'preserved');
assert.equal(maxActive, 1, 'whole-tab read/modify/write operations must be serialized');
failSave = true;
await assert.rejects(store.setTab('bad', true), /synthetic persistence failure/);
await store.setTab('recovered', true);
assert.equal(tab.recovered, true);
const records = ['concurrent-results-and-progress-do-not-overwrite', 'unrelated-tab-data-preserved', 'failed-write-does-not-block-later-writes'];
const pendingWrite = store.setTab('visible', { filled: 84 });
const visible = await store.getTab('visible');
await pendingWrite;
assert.equal(visible?.filled, 84, 'reads must include writes already queued before them');
records.push('reads-wait-for-pending-tab-writes');

const require = createRequire(new URL('../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const library = source.slice(source.indexOf('var __defProp'), source.indexOf('\nconst STYLE = `'))
  .replace('exports2.start = lib.start;', 'globalThis.OCS = exports2; globalThis.EUS = lib; globalThis.testWork = workOrExam$1; exports2.start = lib.start;');
const browser = await chromium.launch({ executablePath: process.argv[2], headless: true });
try {
  const page = await browser.newPage();
  await page.route('**/*', route => route.abort());
  await page.setContent('<main id="questions" style="display:none"></main><div id="results"></div><button id="submit">Submit</button>');
  await page.evaluate(() => {
    window.unsafeWindow = window;
    const values = new Map(), listeners = new Map();
    let tab = { _uid_: 'synthetic' }, nextListener = 0, reads = 0;
    window.GM_listValues = () => Array.from(values.keys());
    window.GM_getValue = (key, fallback) => values.has(key) ? values.get(key) : fallback;
    window.GM_setValue = (key, value) => {
      const before = values.get(key); values.set(key, value);
      for (const { name, callback } of listeners.values()) if (name === key) callback(key, before, value, false);
    };
    window.GM_deleteValue = key => values.delete(key);
    window.GM_addValueChangeListener = (name, callback) => { listeners.set(++nextListener, { name, callback }); return nextListener; };
    window.GM_removeValueChangeListener = id => listeners.delete(id);
    window.GM_getTab = callback => {
      const snapshot = structuredClone(tab);
      setTimeout(() => callback(snapshot), reads++ % 5 === 0 ? 9 : 1);
    };
    window.GM_saveTab = value => { tab = structuredClone(value); };
    window.readProgress = () => structuredClone(tab['common.work-results.rust-progress']);
    window.metrics = { requests: 0, submits: 0, done: false };
    document.getElementById('submit').onclick = () => window.metrics.submits++;
  });
  await page.addScriptTag({ content: library });
  await page.evaluate(() => {
    const { OCS, EUS } = window;
    EUS.$message.info = () => {};
    EUS.$message.error = value => { window.metrics.error = String(value); };
    OCS.CommonProject.scripts.settings.cfg = { rustFastFill: true, rustFillBatchSize: 16, disabledAnswererWrapperNames: [] };
    OCS.CommonProject.scripts.apps.methods.searchAnswerInCaches = (_, fallback) => fallback();
    OCS.CommonProject.scripts.apps.methods.addQuestionCacheFromWorkResult = () => {};
    const ui = OCS.CommonProject.scripts.workResults;
    ui.cfg = { type: 'numbers', currentResultIndex: 0, totalQuestionCount: 0, requestedCount: 0, resolvedCount: 0 };
    document.getElementById('results').append(ui.methods.createWorkResultsPanel());
    for (let index = 0; index < 85; index++) {
      const type = index < 40 ? 0 : index < 55 ? 1 : 3;
      const root = document.createElement('section'); root.className = 'questionLi';
      root.innerHTML = `<h3><span>${index + 1}.</span><span>Question</span>Synthetic ${index}</h3><input name="type${index}" value="${type}"><input type="hidden" name="answer${index}" value=""><div class="stem_answer"><div class="answerBg"></div></div>`;
      const labels = type === 3 ? ['正确', '错误'] : ['Alpha', 'Beta', 'Gamma', 'Delta'];
      for (const [letter, label] of labels.entries()) {
        const option = document.createElement('div'); option.innerHTML = `<i></i><p class="answer_p">${label}</p>`;
        option.querySelector('p').onclick = () => {
          option.querySelector('i').className = 'check_answer';
          root.querySelector('input[name^="answer"]').value += String.fromCharCode(65 + letter);
        };
        root.querySelector('.answerBg').append(option);
      }
      document.getElementById('questions').append(root);
    }
    window.GM_xmlhttpRequest = request => {
      let response;
      if (request.url.endsWith('/status')) response = { implementation: 'rust', status: 'running', inflight: 0 };
      else {
        const question = JSON.parse(request.data), index = Number(question.title.split(' ').at(-1));
        const first = window.metrics.requests++ === 0;
        response = { code: index === 14 ? 0 : 1, msg: 'synthetic ambiguous question', question: question.title,
          answer: question.type === 'multiple' ? 'Alpha#Gamma' : question.type === 'judgement' ? '正确' : 'Alpha', cached: !first, batch_size: 85 };
      }
      queueMicrotask(() => request.onload({ status: 200, responseText: JSON.stringify(response) }));
      return { abort() {} };
    };
    const wrapper = { name: 'synthetic AI', url: 'http://127.0.0.1:18766/t/synthetic/answer', method: 'post', type: 'GM_xmlhttpRequest',
      data: { title: '${title}', type: '${type}' }, handler: 'return r => { if(r.code !== 1) throw new Error(r.msg); return [r.question,r.answer]; }' };
    const worker = window.testWork('exam', { answererWrappers: [wrapper], period: 0, thread: 1,
      redundanceWordsText: '', answerSeparators: '#', preview_mode: true });
    worker.on('done', () => { window.metrics.done = true; });
  });
  await page.waitForFunction(() => window.metrics.done || window.metrics.error, undefined, { timeout: 25000 });
  await page.waitForFunction(() => {
    return window.readProgress()?.phase === 'done' &&
      document.querySelectorAll('.search-infos-num.finish').length === 84;
  }, undefined, { timeout: 5000 });
  const final = await page.evaluate(async () => ({ ...window.metrics, progress:window.readProgress(),
    stored:(await window.OCS.CommonProject.scripts.workResults.methods.getResults()).map(item => ({finish:item.finish,resolved:item.resolved})),
    answerValues:Array.from(document.querySelectorAll('#questions .questionLi input[name^="answer"]')).map(input => input.value),
    filled:Array.from(document.querySelectorAll('#questions .questionLi input[name^="answer"]')).filter(input => input.value).length,
    text:document.getElementById('results').innerText,
    finishedNumbers:document.querySelectorAll('.search-infos-num.finish').length,
    unresolvedNumbers:Array.from(document.querySelectorAll('.search-infos-num')).filter(node => !node.classList.contains('finish') && !node.classList.contains('error')).length
  }));
  assert(!final.error, final.error);
  assert.equal(final.requests, 85); assert.equal(final.submits, 0);
  assert.equal(final.filled, 84, JSON.stringify({ answers:final.answerValues,results:final.stored,progress:final.progress }));
  assert.equal(final.finishedNumbers, 84); assert.equal(final.unresolvedNumbers, 0);
  assert.equal(final.progress.filled, 84); assert.equal(final.progress.resolved, 85);
  assert.equal(final.stored[14].finish, false);
  assert.match(final.text, /已处理: 85\/85/);
  assert(final.stored.every(item => item.resolved));
  records.push('85-question-real-adapter-with-delayed-GM-storage', 'actual-options-results-and-progress-agree', 'uncertain-answer-remains-unfilled', 'no-submission');
  console.log(JSON.stringify({ realChromium:true, realModelCalls:0, passed:records }, null, 2));
} finally { await browser.close(); }
