import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../../', import.meta.url));
const require = createRequire(new URL('../../packages/app/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const browser = resolve(process.argv[2] || process.env.OCS_TEST_CHROMIUM || chromium.executablePath());
if (!existsSync(browser)) {
  throw new Error('Pass a Chromium executable to pnpm test:userscript, set OCS_TEST_CHROMIUM, or run pnpm --filter @ocs-desktop/app exec playwright-core install chromium');
}
const reports = resolve(root, '.cache/userscript-tests');
const suites = [
  ['--check', 'assets/ocs-rust.user.js'],
  ['tests/oracle/test_ocs_timeout.mjs', 'assets/ocs-rust.user.js'],
  ['tests/work-config.mjs', browser],
  ['tests/work-result-storage.mjs', browser],
  ['tests/work-state-lifecycle.mjs', browser],
  ['tests/work-progress.mjs', browser, reports],
  ['tests/fast-fill.mjs', browser, reports]
];
for (const args of suites) {
  console.log(`\nRunning ${args[0]} ${args[0] === '--check' ? args[1] : ''}`);
  const result = spawnSync(process.execPath, args, { cwd: root, stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
}
console.log('\nAll userscript regressions passed (synthetic pages, no real AI or course submissions).');
