import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
const cargo = join(homedir(), '.cargo', 'bin');
const skipTests = process.argv.includes('--skip-tests');
const env = {...process.env, PATH: process.env.PATH + (process.platform==='win32'?';':':') + cargo};
function run(command,args){const r=spawnSync(command,args,{stdio:'inherit',env,shell:process.platform==='win32'&&/\.(cmd|bat)$/i.test(command)});if(r.error)console.error(r.error);if(r.status!==0)process.exit(r.status||1);}
run(process.execPath,['scripts/rust/build-adapter.mjs']);
run(process.execPath,['scripts/rust/build-settings.mjs']);
if (!skipTests) run(process.execPath,['tests/settings-smoke.cjs']);
const pnpm=process.platform==='win32'?'pnpm.cmd':'pnpm';
run(pnpm,['--dir','packages/web','exec','vue-tsc','--noEmit']);
run(pnpm,['--dir','packages/web','exec','vite','build']);
if (!skipTests) run('cargo',['test','-p','ocs-core','--locked']);
run('cargo',['build','-p','ocs-core','--release','--locked']);
run(pnpm,['exec','tauri','build','--bundles',process.platform==='darwin'?'app':process.platform==='win32'?'nsis':'appimage']);
if (process.platform === 'darwin') {
 const app = 'target/release/bundle/macos/OCS Rust.app';
 if (!process.env.APPLE_SIGNING_IDENTITY) run('codesign',['--force','--sign','-','--timestamp=none',app]);
 run('codesign',['--verify','--deep','--strict',app]);
}
