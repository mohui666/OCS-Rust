import { initialize } from './utils/native';
const pending = document.getElementById('app')!;
pending.style.cssText='padding:32px;font:16px system-ui';
pending.textContent='正在启动 OCS Rust… 若出现系统钥匙串提示，请在系统窗口处理。';
initialize().then(()=>{pending.removeAttribute('style');return import('./main');}).catch(error=>{
 const app=document.getElementById('app')!;
 app.style.cssText='padding:48px;font:16px system-ui;white-space:pre-wrap';
 app.textContent='OCS Rust 无法启动\n\n'+String(error)+'\n\n原配置文件已保留。修复错误后请重新打开。';
});
