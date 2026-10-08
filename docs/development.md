# 开发、构建与发布

## 架构和目录

| 路径 | 作用 |
| --- | --- |
| `crates/ocs-core` | 题库协议、AI API、Codex 子进程、缓存、存储、迁移及工作进程管理 |
| `src-tauri` | Rust / Tauri 桌面、本地 API 和原生命令 |
| `packages/web` | Vue 界面和脚本内嵌 AI 表单 |
| `native-adapter` | Rust 管理的 Node / Playwright 适配器，复用 `packages/app` 的部分工作进程代码 |
| `assets/ocs-rust.user.js` | 独立发行的用户脚本，也是桌面设置库的来源 |
| `assets/bridge` | 页面采集、提示词、输出 schema 和请求模板 |
| `tests` | 合成页面检查及历史 Python 对照实现，不用于产品运行 |
| `scripts/rust` | 构建、版本检查和分包 |
| `docs/upstream` | 上游 Electron 更新记录及旧工作流，仅保留历史参考 |

旧 Electron 桌面入口保留在源码中，新版桌面运行时不启动 Electron 或 Python。Vue 和网页脚本仍使用 JavaScript / TypeScript；不要把“Rust 重构”理解为所有代码都改写成 Rust。

## 环境与命令

使用 Rust stable、Node.js 22 或 24、pnpm 10（项目固定版本为 10.21.0）。本版在 macOS arm64 / Rust 1.99.0 / Node 24.19.0 和 Windows x64 / Rust 1.97.1 / Node 22.19.0 原生构建；两台本机构建机使用已有 pnpm 11。系统库要求见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)。

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm dev
pnpm build
```

`pnpm dev` 构建适配器与设置库后启动 Tauri。`pnpm build` 依次构建适配器、提取设置库、运行设置模块检查、检查 Vue 类型、构建前端、运行 Rust 测试、构建 CLI 和桌面应用。

只需要构建和类型检查、不运行测试时：

```sh
pnpm build:release
pnpm release:pack
```

macOS 分包输出到 `dist/release/v<桌面版本>/`。Windows 原生构建输出为 `target/release/bundle/nsis/OCS Rust_<桌面版本>_x64-setup.exe`。不要在 Apple Silicon 上直接把文件改名为 Windows、Linux 或 Intel 版；Node 运行时跟随构建机平台和架构一起打包。

合并两平台发行包时，先在两端检出同一发布提交并分别完成原生构建。在 Windows PowerShell 执行：

```powershell
pnpm build
./scripts/rust/prepare-windows-release.ps1
```

该脚本检查 Windows EXE、NSIS 安装程序和随包 Node 的版本与架构，输出 `dist/release/v<桌面版本>/windows-x64/`，内含安装程序及 `windows-build.json`。将整个目录复制到 macOS，在已构建且源码干净的同一提交下执行：

```sh
pnpm release:pack --windows-package /absolute/path/to/windows-x64
```

分包工具核对两端源码提交、桌面与脚本版本、Windows 安装包大小和哈希，然后生成包含两个桌面包的统一清单。Windows 构建记录不包含本机路径、账号或配置。

macOS 应用位置为 `target/release/bundle/macos/OCS Rust.app`，CLI 为 `target/release/ocs-bridge` 和 `target/release/ocs-migrate`。本地构建使用 ad-hoc 签名；设置 `APPLE_SIGNING_IDENTITY` 时保留 Tauri 的指定签名流程。普通构建不等于 Apple 公证。

## 定向检查

按改动选择必要检查，不需要每次运行全部命令：

```sh
node --check assets/ocs-rust.user.js
pnpm --dir packages/web exec vue-tsc --noEmit
cargo check -p ocs-desktop-rust --locked
cargo test -p ocs-core --locked
node tests/oracle/test_ocs_timeout.mjs assets/ocs-rust.user.js
node tests/fast-fill.mjs /absolute/path/to/chromium
node tests/work-progress.mjs /absolute/path/to/chromium
node tests/adapter-smoke.mjs /absolute/path/to/chromium
```

浏览器检查使用临时资料和合成题目，不能代替真实账户、课程提交、评分或跨平台验收。发布范围见 [verification.md](verification.md)。

## 独立题库 CLI

```sh
./target/release/ocs-bridge check /absolute/path/bridge.config.json
./target/release/ocs-bridge serve /absolute/path/bridge.config.json
./target/release/ocs-bridge service install /absolute/path/bridge.config.json
./target/release/ocs-bridge service start
./target/release/ocs-bridge service status
./target/release/ocs-bridge service stop
```

配置可由桌面保存，独立程序支持读取同机加密配置。API 运行时密钥可通过 `OCS_AI_API_KEY` 环境变量提供。服务配置不是公开示例，不要提交真实文件。`service install` 只写服务定义，需要再执行 `start`；不要与桌面服务争用端口。

复制旧资料的命令：

```sh
./target/release/ocs-migrate /absolute/old/config.json /absolute/new/data /absolute/resource/root
```

资源根目录需包含 `assets/ocs-rust.user.js`，macOS 安装包中为 `OCS Rust.app/Contents/Resources`。迁移不会改写旧目录。

## 版本规范

- 桌面版本源：`Cargo.toml` 的 `workspace.package.version`，同步 `src-tauri/tauri.conf.json` 与根 `package.json`。界面版本从 Cargo 包版本读取。
- 脚本版本源：`assets/ocs-rust.user.js` 的 `@version`，使用独立 SemVer。迁移和分包会读取该字段。
- 上游基线只记录在 README / 更新记录中，不再混入发行版本。
- 本版配套：桌面 `0.1.1` + 脚本 `0.1.1`，综合 Release 标签为 `v0.1.1`。仅脚本更新可使用 `userscript-v<版本>` 标签。
- 分包工具检查版本一致性、实际架构、签名，以及包内脚本与源码的哈希，生成独立 ZIP、原始 `.user.js`、清单与 `SHA256SUMS.txt`。

## 发布流程

1. 更新版本、变更记录和相关文档，只提交需要公开的源码与文件。
2. 在目标系统构建，记录本次检查与未验证范围；不要将本机配置、Cookie、日志或备份装进包。
3. 执行 `pnpm release:pack`；同时发行 Windows 时使用上面的 `--windows-package` 流程。检查输出清单、ZIP 内容与安装程序版本。
4. 推送源码和对应标签，在 GitHub 创建 Release，分别上传桌面包、脚本包、原始脚本、清单及校验文件。
5. 检查远端 Release 状态、下载文件名、大小和 SHA-256。

`.github/workflows/build.yml` 仅供手动构建并保留下载工件，不会自动发布。旧 Electron 的自动发布和云存储工作流已移出活动目录。脚本 `scripts/release.sh` 仅负责本地分包，也不会提交或推送。

用户的真实运行记录保留在开发者本机，不随公开源码或发行包提供。公开验证摘要不会包含私人路径、账户、课程信息或访问令牌。
