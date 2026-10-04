import { invoke, snapshot, isNative, events } from './native';
function registerRemote(name:string) {
 return {
  get(property:string):any {
   if(name==='electron-store'&&property==='store') return snapshot().store;
   if(name==='path'&&property==='sep') return snapshot().platform==='win32'?'\\':'/';
   throw new Error('不支持同步属性 '+name+'.'+property);
  },
  async call(property:string,...args:any[]):Promise<any> {
   if(name==='webContents') {
    if(['copy','cut','paste','selectAll','undo','redo','delete'].includes(property)) {
     if(property==='paste') {const text=await invoke('native_call',{op:'clipboard.readText',args:[]});return document.execCommand('insertText',false,text);}
     return document.execCommand(property);
    }
   }
   if(name==='app'&&property==='quit') return events.emit('close');
   if(!isNative&&name==='logger'){console.log(...args);return;}
   return invoke('native_call',{op:name+'.'+property,args});
  },
  callSync(property:string):any {
   if(name==='methods'&&property==='getRawScripts') return snapshot().scripts??[];
   if(name==='methods'&&property==='isEncryptionAvailable') return true;
   throw new Error('不支持同步调用 '+name+'.'+property);
  }
 };
}
export const remote=Object.fromEntries(['electron-store','fs','path','os','crypto','OCSApi','win','webContents','app','dialog','methods','logger','clipboard'].map(n=>[n,registerRemote(n)])) as Record<string,ReturnType<typeof registerRemote>>;
