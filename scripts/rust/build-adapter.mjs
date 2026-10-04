import { build } from 'esbuild';
import { cp, mkdir, writeFile, chmod, readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
const require = createRequire(resolve('packages/app/package.json'));
const dest = resolve('native-adapter/dist');
await mkdir(dest, {recursive:true});
await build({entryPoints:['native-adapter/index.ts'],outfile:dest+'/worker.cjs',bundle:true,platform:'node',format:'cjs',target:'node22',legalComments:'linked',external:['playwright-core']});
await mkdir(dest+'/node_modules', {recursive:true});
await cp(dirname(require.resolve('playwright-core/package.json')),dest+'/node_modules/playwright-core',{recursive:true});
const nodeName = process.platform === 'win32' ? 'node.exe' : 'node';
await cp(process.execPath,dest+'/'+nodeName); await chmod(dest+'/'+nodeName,0o755);
let nodeLicense;
try { nodeLicense = await readFile(`assets/licenses/Node-${process.version}.LICENSE`); }
catch {
 const response = await fetch(`https://raw.githubusercontent.com/nodejs/node/${process.version}/LICENSE`);
 if (!response.ok) throw new Error(`Cannot obtain license for the bundled Node ${process.version}`);
 nodeLicense = await response.text();
}
await writeFile(dest+'/Node-LICENSE',nodeLicense);
const metadata = execFileSync(process.execPath,[dest+'/worker.cjs','--metadata'],{encoding:'utf8'});
JSON.parse(metadata); await writeFile(dest+'/scripts.json',metadata);
await writeFile(dest+'/runtime.json',JSON.stringify({node:process.version,platform:process.platform,arch:process.arch,playwright:require('playwright-core/package.json').version},null,2));
console.log('Browser adapter built: '+dest);
