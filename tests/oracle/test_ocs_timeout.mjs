import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(process.argv[2] || new URL('./ocs-unlimited.user.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const handlerStart = source.indexOf('  async function defaultAnswerWrapperHandler(');
const handlerEnd = source.indexOf('\n    return searchInfos;\n  }', handlerStart);
assert.ok(handlerStart >= 0 && handlerEnd > handlerStart);
const handlerCode = source.slice(handlerStart, handlerEnd + '\n    return searchInfos;\n  }'.length);
const workerStart = source.indexOf('      const waitForRequested = ');
const workerEnd = source.indexOf('      const resolverThread = ', workerStart);
assert.ok(workerStart >= 0 && workerEnd > workerStart);
const workerCode = source.slice(workerStart, workerEnd);

for (const seconds of [0, 180]) {
  const timers = [];
  let reply;
  const context = {
    URL, console: { error() {} },
    CommonProject: { scripts: { settings: { cfg: { disabledAnswererWrapperNames: [] } } } },
    AnswerWrapperHandlerConfig: { timeout_seconds: seconds },
    rustFastFill: { observe: async () => {} },
    request: () => new Promise(resolve => { reply = resolve; }),
    $: { sleep: ms => new Promise(resolve => timers.push({ ms, resolve })) }
  };
  const handler = vm.runInNewContext(handlerCode + '\ndefaultAnswerWrapperHandler', context);
  const pending = handler([{ name: 'test', url: 'http://127.0.0.1/answer', method: 'post',
    data: { title: '${title}' }, handler: 'return res => [res.question, res.answer]' }], { title: 'test' });
  if (seconds === 0) {
    assert.equal(timers.length, 0, 'unlimited search must not create a deadline');
    // Advance beyond both former limits before letting the remote answer finish.
    for (const timer of timers) if (timer.ms <= 610_000) timer.resolve();
    reply({ question: 'test', answer: 'success after 610 virtual seconds' });
    assert.equal((await pending)[0].results[0].answer, 'success after 610 virtual seconds');
  } else {
    assert.equal(timers[0].ms, 180_000);
    timers[0].resolve();
    assert.match((await pending)[0].error, /题库请求超时/);
  }
}

for (const seconds of [0, 180]) {
  const timers = [], polls = [];
  const context = {
    AnswerWrapperHandlerConfig: { timeout_seconds: seconds },
    setInterval: fn => { polls.push(fn); return 1; }, clearInterval() {}, clearTimeout() {},
    setTimeout: (fn, ms) => { timers.push({ fn, ms }); return 2; }
  };
  const wait = vm.runInNewContext(`(() => { ${workerCode} return waitForRequested; })()`, context);
  const result = { requested: false };
  const pending = wait(result);
  if (seconds === 0) {
    assert.equal(timers.length, 0, 'unlimited worker must not abandon the request');
    for (const timer of timers) if (timer.ms <= 610_000) timer.fn();
    result.requested = true;
    polls[0]();
    await pending;
  } else {
    assert.equal(timers[0].ms, 190_000);
    const check = assert.rejects(pending, /答题超时/);
    timers[0].fn();
    await check;
  }
}
console.log('PASS: OCS search and worker wait beyond 610 virtual seconds; finite limits still work.');
