import { remote } from './remote';
import { invoke, events } from './native';
import { lang, store } from '../store';
import type { LaunchOptions } from 'playwright-core';
import { reactive } from 'vue';
import type { Browser } from '../fs/browser';
import { Message } from '@arco-design/web-vue';
import EventEmitter from 'events';
import { notify } from './notify';
import { Status } from './statusBar';
import { filterScriptsNeedingInstall } from './script-version';
export type RemoteScriptWorker = (event: string, ...args: any[]) => Promise<any>;
export class Process extends EventEmitter {
 uid: string;
 worker: RemoteScriptWorker;
 status: 'closed'|'closing'|'launching'|'launched' = 'closed';
 browser: Browser;
 launchOptions: LaunchOptions;
 logs: string[] = [];
 video: HTMLVideoElement | undefined;
 stream: MediaStream | undefined;
 private listener?: (...args: any[]) => void;
 static from(uid:string) { return processes.find(p=>p.uid===uid); }
 static remove(uid:string) { const i=processes.findIndex(p=>p.uid===uid); if(i>=0) processes.splice(i,1); }
 constructor(browser:Browser,options:LaunchOptions) {
  super(); this.browser=browser; this.uid=browser.uid; this.launchOptions=options;
  this.worker=(event,...args)=>invoke('worker_call',{uid:this.uid,event,args});
 }
 async init(onConsole?: (data:any)=>void) {
  this.listener=(_event:any, data:any)=>{
   const args=data.args||[];
   if(data.event==='launched') this.status='launched';
   if(data.event==='log'||data.event==='worker-error') {
    const text=String(args[0]??'');this.logs.push(text);if(this.logs.length>2000)this.logs.shift();
    onConsole?.(text); this.emit('log',text+'\r\n');
    if(data.event==='worker-error') notify(this.browser.name+' 错误',text,this.uid,{type:'error',copy:true});
   }
   if(data.event==='browser-closed'||data.event==='exit') {
    this.status='closed'; this.stream?.getTracks().forEach(t=>t.stop());
    Process.remove(this.uid);Status.clear();
    if(data.event==='exit'&&this.listener)events.removeListener('worker:'+this.uid,this.listener);
   }
   this.emit(data.event,...args);
  };
  events.on('worker:'+this.uid,this.listener);
  try {
   await invoke('worker_start',{uid:this.uid});
   await this.worker('init',{store,cachePath:this.browser.cachePath,uid:this.uid,automationScripts:this.browser.automationScripts,browserInfo:{name:this.browser.name,notes:this.browser.notes,tags:this.browser.tags},config:{enable_dialog:store.render.setting.browser.enableDialog},langs:store.render.langs});
  } catch(error) {await invoke('worker_stop',{uid:this.uid}).catch(()=>{});events.removeListener('worker:'+this.uid,this.listener);Process.remove(this.uid);throw error;}
 }
	async launchPreCheck() {
		// 检查
		if (!this.launchOptions.executablePath) {
			Message.error('浏览器路径为空，请在软件设置中修改');
			return;
		}

		try {
			const exists = await remote.fs.call('existsSync', this.launchOptions.executablePath);
			if (!exists) {
				Message.error('浏览器路径不存在，请在软件设置中修改');
				return;
			}

			// 脚本检查
			Status.loading('正在检查本地脚本...');
			const enabledUserScripts = store.render.scripts.filter((s) => s.enable);
			for (const s of enabledUserScripts) {
				if (!s.url.startsWith('http')) {
					const res = await remote.fs.call('existsSync', s.info?.code_url || s.url);
					if (!res) {
						notify(
							'本地脚本不存在',
							lang('error_when_script_not_found', `本地脚本 ${s.info?.name}：(${s.url})\n不存在，请检查脚本路径`, {
								name: s.info?.name || '',
								url: s.url
							}),
							'process_launch_error_' + s.url,
							{
								duration: 60 * 1000,
								type: 'warning',
								copy: true
							}
						);
					}
				}
			}

			Status.loading('正在检查脚本更新...');
			const scriptsToInstall = await filterScriptsNeedingInstall(enabledUserScripts);
			if (scriptsToInstall.length > 0) {
				Status.loading(`正在启动 ${this.browser.name}（需更新/安装 ${scriptsToInstall.length} 个脚本）...`);
			} else {
				Status.loading(`正在启动 ${this.browser.name}（脚本均为最新，无需更新）...`);
			}
			this.once('launched', () => {
				// 安装成功后更新 lastInstalledVersion
				for (const item of scriptsToInstall) {
					item.script.lastInstalledVersion = item.latestVersion;
				}
				Status.clear();
			});
			this.once('exit', () => {
				Status.clear();
			});
			return { scriptsToInstall, enabledScriptCount: enabledUserScripts.length };
		} catch (err) {
			Message.error('浏览器路径读取错误 : ' + String(err));
		}
	}

 async launch() {
  this.status='launching';
  try {
   const result=await this.launchPreCheck();
   if(!result) throw new Error('浏览器启动检查失败');
   await this.worker('launch',{
    userDataDir:this.browser.cachePath,
    enabledScriptCount:result.enabledScriptCount,
    userscripts:result.scriptsToInstall.map(item=>item.script.isLocalScript
     ? 'http://localhost:'+store.server.port+'/api/local-userscript?path='+encodeURIComponent(item.script.info?.code_url||item.script.url)+'&token='+encodeURIComponent(store.server.authToken)
     : item.script.info?.code_url||item.script.url),
    ...this.launchOptions
   });
   this.status='launched';
  } catch(error) {await invoke('worker_stop',{uid:this.uid});Process.remove(this.uid);throw error;}
 }
 async close() {this.status='closing';await invoke('worker_stop',{uid:this.uid});this.status='closed';this.stream?.getTracks().forEach(t=>t.stop());Process.remove(this.uid);}
 bringToFront() {if(this.status==='launched')this.worker('bringToFront').catch(e=>Message.error(String(e)));else Message.warning('必须先启动文件');}
 toString(){return '[Process]';}
}
export const processes: Process[] = reactive([]);
