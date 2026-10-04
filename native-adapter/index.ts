import { createInterface } from 'node:readline';
import { inspect } from 'node:util';
import { ScriptWorker } from '../packages/app/src/worker';
import { AutomationScripts } from '../packages/app/src/scripts';
const write = (value: unknown) => process.stdout.write(JSON.stringify(value) + '\n');
if (process.argv.includes('--metadata')) { write(JSON.parse(JSON.stringify(AutomationScripts))); process.exit(0); }
process.send = ((value: unknown) => { write(value); return true; }) as typeof process.send;
for (const level of ['log','info','warn','error','debug'] as const) {
 console[level] = (...args: unknown[]) => write({ event: level === 'error' ? 'worker-error' : 'log', args: [args.map(a => typeof a === 'string' ? a : inspect(a, {depth:3})).join(' ')] });
}
const worker = new ScriptWorker();
const allowed = new Set(['init','launch','close','bringToFront','gotoWebRTCPage','closeWebRTCPage','kill','snapshot','goto']);
const reader = createInterface({ input: process.stdin });
reader.on('line', async line => {
 let id: string | undefined;
 try {
  const r = JSON.parse(line); id = r.id;
  if (!allowed.has(r.event) || !Array.isArray(r.args)) throw new Error('Unsupported browser command');
  const result = r.event === 'snapshot'
   ? await Promise.all((worker.browser?.pages() ?? []).map(async p => ({url:p.url(),title:await p.title()})))
   : await (worker as any)[r.event](...r.args);
  if (r.event === 'launch' && !worker.browser) throw new Error('浏览器初始化失败，请查看日志中的具体错误');
  write({id, result: result ?? null});
 } catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  write({id,error:message}); write({event:'worker-error',args:[message]});
 }
});
reader.on('close', () => worker.close().finally(() => process.exit()));
process.on('SIGTERM', () => worker.close().finally(() => process.exit()));
process.on('unhandledRejection', error => console.error(error));
