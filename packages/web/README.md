# OCS Rust 界面

Vue 3 + Arco Design + Vite。用于 Tauri 桌面、本地导航页和课程脚本内嵌 AI 设置页。

从仓库根目录运行：

```sh
pnpm build:settings
pnpm --dir packages/web exec vue-tsc --noEmit
pnpm --dir packages/web exec vite build
```

完整开发入口为根目录的 `pnpm dev`。生成的设置库位于 `src/generated`，应修改 `assets/ocs-rust.user.js` 后重新生成，不要手改构建产物。

参见 [开发文档](../../docs/development.md)。
