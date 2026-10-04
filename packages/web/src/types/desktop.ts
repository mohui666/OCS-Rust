/** Serializable contracts owned by the Rust desktop boundary. */
export interface Config {
 label:string;value:any;hide?:boolean;type?:'text'|'password'|'number'|'textarea'|'select'|'switch';required?:boolean;max?:number;min?:number;options?:{label:string;value:any}[];placeholder?:string;
}
export interface RawAutomationScript {name:string;icon?:string;configs:Record<string,Config>}
export interface UserScripts {id:number;url:string;enable:boolean;info:any;isLocalScript:boolean;isInternetLinkScript:boolean;lastInstalledVersion?:string;lastInfoUpdateTime?:number}
export interface AppStore {
 name:string;version:string;render:unknown;
 paths:{'app-path':string;'user-data-path':string;'exe-path':string;'logs-path':string;'config-path':string;userDataDirsFolder:string;downloadFolder:string;extensionsFolder:string};
 app:{video_frame_rate:number};window:{alwaysOnTop:boolean;autoLaunch:boolean};server:{port:number;authToken:string};
}
