import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import {mkdtemp,mkdir,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {createServer} from 'node:http';
import assert from 'node:assert/strict';
const executable=process.argv[2];
if(!executable)throw new Error('Pass an existing Chromium executable path');
const root=await mkdtemp(join(tmpdir(),'ocs-rust-adapter-test-'));
for(const n of ['extensions','logs','profile'])await mkdir(join(root,n));
const server=createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>OCS synthetic fixture</title><h1>OCS fixture</h1>');});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;
const child=spawn(process.execPath,[resolve('native-adapter/dist/worker.cjs')],{stdio:['pipe','pipe','pipe']});
const pending=new Map();const events=[];let errors='';
createInterface({input:child.stdout}).on('line',line=>{const v=JSON.parse(line);if(v.id){const p=pending.get(v.id);if(p){pending.delete(v.id);v.error?p.reject(new Error(v.error)):p.resolve(v.result);}}else events.push(v);});
child.stderr.on('data',b=>errors+=b);
child.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Adapter exited'));pending.clear();});
let serial=0;
async function call(event,...args){const id=String(++serial);return new Promise((resolve,reject)=>{pending.set(id,{resolve,reject});child.stdin.write(JSON.stringify({id,event,args})+'\n');});}
const timeout=setTimeout(()=>{child.kill('SIGKILL');throw new Error('Adapter smoke timed out '+errors)},25000);
try{
 await call('init',{store:{paths:{extensionsFolder:join(root,'extensions'),'logs-path':join(root,'logs')},server:{port,authToken:'fixture-only-token'}},uid:'fixture-browser',cachePath:join(root,'profile'),automationScripts:[],browserInfo:{name:'Fixture',notes:'',tags:[]},config:{enable_dialog:false},langs:{}});
 await call('launch',{executablePath:executable,headless:false,args:[],userDataDir:join(root,'profile'),userscripts:[],enabledScriptCount:0});
 assert(events.some(e=>e.event==='launched'),'launched event');
 let pages=await call('snapshot');assert(pages.some(p=>p.url.includes('index.html')),'local bookmark page');
 await call('gotoWebRTCPage');pages=await call('snapshot');assert(pages.some(p=>p.title==='fixture-browser'),'monitor window title');
 await call('closeWebRTCPage');pages=await call('snapshot');assert(!pages.some(p=>p.title==='fixture-browser'),'monitor page cleanup');
 await call('bringToFront');
 const exited=new Promise(resolve=>child.once('exit',resolve));child.stdin.end();await exited;
 assert(events.some(e=>e.event==='browser-closed'),'browser closed event');
 console.log(JSON.stringify({passed:true,checks:['launch','bookmark-page','monitor-identification','monitor-cleanup','focus','close'],realChromium:true,modelCalls:0}));
}finally{clearTimeout(timeout);child.kill('SIGTERM');server.close();await rm(root,{recursive:true,force:true});}
