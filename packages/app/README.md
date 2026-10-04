# 上游桌面代码与浏览器适配器

此目录保留 OCS Desktop 的 Electron 代码，并提供当前 Rust 版复用的浏览器工作进程与脚本适配代码。

当前桌面入口是 `src-tauri`；默认开发和构建从仓库根目录执行 `pnpm dev` / `pnpm build`。不要使用旧 Electron 发布流程给 Rust 版打包。

参见 [架构与构建](../../docs/development.md)。
