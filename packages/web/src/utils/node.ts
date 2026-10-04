import { isNative, events, invoke } from './native';
export const inBrowser = !isNative;
// Compatibility names only. No Electron or Node runtime is loaded by the Vue UI.
export const electron = {
 ipcRenderer: events,
 shell: {openPath:(path:string)=>invoke('native_call',{op:'shell.openPath',args:[path]}),openExternal:(url:string)=>invoke('native_call',{op:'shell.openExternal',args:[url]})},
 clipboard:{writeText:(text:string)=>invoke('native_call',{op:'clipboard.writeText',args:[text]})}
};
