# OCS Rust

OCS Desktop 的 Rust / Tauri 分支，提供浏览器管理、课程用户脚本和本地 AI 题库。保留 Vue 界面与 JavaScript 网页脚本，桌面主进程、题库服务、配置加密、迁移与资源管理使用 Rust。

这是独立维护的社区分支，不是 OCS 官方发行版。上游桌面基线为 [OCS Desktop 2.12.0](https://github.com/ocsjs/ocs-desktop/tree/ecc6bb7ee79cb713caab7e896a913383e9437a14)，用户脚本基线为 [OCS 4.15.3](https://github.com/ocsjs/ocsjs)。

## 下载

前往 **[GitHub Releases](https://github.com/mohui666/OCS-Rust/releases/latest)**。桌面和脚本分别提供下载、分别维护版本。

| 文件 | 用途 |
| --- | --- |
| `OCS-Rust-Desktop-0.1.1-windows-x64-setup.exe` | Windows x64 安装程序，本次在 Windows 11 实机验证 |
| `OCS-Rust-Desktop-0.1.1-macos-arm64.zip` | Apple Silicon Mac 桌面应用，解压后拖入「应用程序」 |
| `OCS-Rust-Userscript-0.1.1.zip` | 用户脚本、安装说明与许可证 |
| `ocs-rust.user.js` | 可直接交给 ScriptCat / Tampermonkey 安装的脚本 |
| `SHA256SUMS.txt` | 下载文件的 SHA-256 校验值 |
| `release-manifest.json` | 桌面版本、脚本版本、源码提交和产物信息 |

本版提供 **Windows x64** 和 **macOS arm64**，不提供 Linux 或 Intel Mac 二进制。Windows 安装包未做代码签名；macOS 包使用本地 ad-hoc 签名，**未经过 Apple Developer ID 签名或公证**。已执行的检查和未验证功能见[验证范围](docs/verification.md)。

桌面 `0.1.1` 随附用户脚本 `0.1.1`，与独立脚本包内容一致，用于初始化和设置界面；脚本包可单独下载和更新，不包含桌面程序、浏览器或任何账户配置。

## 开始使用

1. 安装并打开 OCS Rust，从 **工具 → 初始化设置** 准备兼容的 Chromium 与脚本管理扩展。启动时不再自动弹出公告、教程或初始化窗口。
2. 启动 OCS 管理的浏览器，登录课程平台并选择课程。当前使用有窗口浏览器。
3. 在脚本中打开 **⚙️ 全局设置 → AI / 其他题库设置**。
4. 选择 **GPT 登录态**或 **API Key**，选择模型并保存。使用 AI 时再打开「使用 AI 答题」。其他题库在旁边的页签管理。

AI 服务随应用就绪；只有启用 AI 答题并实际搜题才调用模型。打开设置、保存参数、读取模型列表不会发起答题。API Key 加密保存在本机，不需要手工拼写 AI 题库 JSON。GPT 模式使用用户已有的 Codex CLI 登录，不部署本地模型。

- [桌面使用、AI 配置、迁移与常见问题](docs/guide.md)
- [脚本独立安装与更新](docs/userscript.md)
- [开发、构建、版本与发布](docs/development.md)
- [验证范围与已知限制](docs/verification.md)
- [更新记录](CHANGELOG.md)

## 功能

- Rust / Tauri 桌面与本机回环题库服务，支持浏览器资料隔离、配置导入导出和旧资料复制迁移。
- GPT 登录态与 OpenAI 兼容 API 两种题库方式；模型选择、推理强度和高级参数统一配置。
- 整页查询、重复请求合并与缓存；已有答案默认每批填写 16 题，可在 1～32 之间调整。
- 显示请求、返回、填入数量和等待状态；计数不等于平台保存、提交或评分结果。
- 保留登录资料和上次页面恢复选项；导航页保留六个网课入口，图标随软件打包。

## 从源码构建

需要 Rust stable、Node.js 22 或 24、pnpm 10，以及目标系统的 [Tauri 2 构建依赖](https://v2.tauri.app/start/prerequisites/)。

```sh
git clone https://github.com/mohui666/OCS-Rust.git
cd OCS-Rust
pnpm install --frozen-lockfile --ignore-scripts
pnpm dev
```

`pnpm build` 执行构建和已有测试；`pnpm build:release` 只做构建和类型检查。Windows 构建生成 NSIS 安装程序；macOS 构建后执行 `pnpm release:pack`，生成分开的桌面包、脚本包及校验清单，也可合并 Windows 原生构建的安装包。详细流程见[开发文档](docs/development.md)。

## 许可与归属

项目代码采用 [MIT](LICENSE)，保留 enncy 及上游贡献者署名。第三方组件、运行时和站点图标各自的许可与归属见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目与课程平台、OpenAI 及 OCS 上游没有官方关联。请按课程和平台规则使用。
