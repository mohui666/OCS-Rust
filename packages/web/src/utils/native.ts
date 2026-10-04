type NativeWindow = Window & { __TAURI__?: any; __OCS_BOOTSTRAP__?: any };
const host = window as NativeWindow;
export const isNative = !!host.__TAURI__;
export const snapshot = () => host.__OCS_BOOTSTRAP__ ?? {};
export async function invoke<T = any>(command: string, args: Record<string,unknown> = {}): Promise<T> {
 if (!isNative) throw new Error('此功能需要在 OCS Rust 桌面窗口中使用');
 return host.__TAURI__.core.invoke(command,JSON.parse(JSON.stringify(args)));
}
type Listener = (...args: any[]) => void;
const listeners = new Map<string,Set<Listener>>();
export const events = {
 on(name:string,fn:Listener) { if (!listeners.has(name)) listeners.set(name,new Set()); listeners.get(name)!.add(fn); return events; },
 once(name:string,fn:Listener) { const once = (...args:any[]) => {events.removeListener(name,once);fn(...args)}; return events.on(name,once); },
 removeListener(name:string,fn:Listener) {listeners.get(name)?.delete(fn);return events;},
 emit(name:string,...args:any[]) { for (const fn of Array.from(listeners.get(name) ?? [])) fn({},...args); }
};
export async function initialize() {
 if (!isNative) return;
 await host.__TAURI__.event.listen('native-event',(e:any)=>events.emit(e.payload.event,...e.payload.args));
 await host.__TAURI__.event.listen('worker-event',(e:any)=>events.emit('worker:'+e.payload.uid,e.payload));
 host.__OCS_BOOTSTRAP__=await invoke('bootstrap');
}
