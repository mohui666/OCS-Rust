import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '../..');
const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--windows-package')) {
	throw new Error('用法：pnpm release:pack [--windows-package <Windows 分包目录>]');
}
const command = (name: string, args: string[]) => execFileSync(name, args, { cwd: root, encoding: 'utf8' }).trim();
const readJson = async (path: string) => JSON.parse(await readFile(path, 'utf8'));
const sha256 = async (path: string) => {
	const hash = createHash('sha256');
	for await (const chunk of createReadStream(path)) hash.update(chunk);
	return hash.digest('hex');
};
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?$/;
if (process.platform !== 'darwin') throw new Error('当前分包工具仅支持 macOS 原生构建。');
if (command('git', ['status', '--porcelain']).length) throw new Error('请先提交本次公开源码，再生成带源码提交号的发行包。');

const config = await readJson(join(root, 'src-tauri/tauri.conf.json'));
const desktopVersion: string = config.version;
const packageJson = await readJson(join(root, 'package.json'));
const cargo = await readFile(join(root, 'Cargo.toml'), 'utf8');
const cargoVersion = cargo.match(/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/)?.[1];
if (!semver.test(desktopVersion) || packageJson.version !== desktopVersion || cargoVersion !== desktopVersion) {
	throw new Error('桌面版本必须是 SemVer，并在 Cargo、Tauri 和根 package.json 中保持一致。');
}
const script = join(root, 'assets/ocs-rust.user.js');
const scriptSource = await readFile(script, 'utf8');
const scriptVersion = scriptSource.match(/^\/\/\s*@version\s+(\S+)\s*$/m)?.[1];
if (!scriptVersion || !semver.test(scriptVersion)) throw new Error('用户脚本 @version 必须使用独立 SemVer。');
command(process.execPath, ['--check', script]);

const app = join(root, 'target/release/bundle/macos/OCS Rust.app');
const resources = join(app, 'Contents/Resources');
const bundledVersion = command('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleShortVersionString', join(app, 'Contents/Info.plist')]);
if (bundledVersion !== desktopVersion) throw new Error('应用包版本过期，请先重新构建。');
const runtime = await readJson(join(resources, 'adapter/runtime.json'));
if (runtime.platform !== process.platform || runtime.arch !== process.arch) throw new Error('打包目标与当前平台架构不一致。');
if (await sha256(script) !== await sha256(join(resources, 'assets/ocs-rust.user.js'))) throw new Error('包内用户脚本与当前源码不一致。');
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) {
	if (await sha256(join(root, name)) !== await sha256(join(resources, name))) throw new Error(`应用包缺少当前许可文件：${name}`);
}
command('codesign', ['--verify', '--deep', '--strict', app]);

const out = join(root, 'dist/release', `v${desktopVersion}`);
await mkdir(out, { recursive: true });
const sourceCommit = command('git', ['rev-parse', 'HEAD']);
const targets = [{ platform: runtime.platform, arch: runtime.arch, node: runtime.node, playwright: runtime.playwright }];
const additionalFiles: string[] = [];
if (args.length) {
	const folder = resolve(args[1]);
	const windows = await readJson(join(folder, 'windows-build.json'));
	const expectedName = `OCS-Rust-Desktop-${desktopVersion}-windows-x64-setup.exe`;
	if (windows.project !== 'OCS Rust' || windows.sourceCommit !== sourceCommit || windows.desktopVersion !== desktopVersion ||
		windows.scriptVersion !== scriptVersion || windows.target?.platform !== 'win32' || windows.target?.arch !== 'x64' ||
		windows.file?.name !== expectedName || windows.userscriptSha256 !== await sha256(script)) {
		throw new Error('Windows 分包的源码提交、版本、平台或脚本与当前发布不一致。');
	}
	const installer = join(folder, expectedName);
	if ((await stat(installer)).size !== windows.file.bytes || await sha256(installer) !== windows.file.sha256) {
		throw new Error('Windows 安装包大小或 SHA-256 与原生构建记录不一致。');
	}
	const destination = join(out, expectedName);
	await cp(installer, destination);
	additionalFiles.push(destination);
	targets.push(windows.target);
}
const desktopZip = join(out, `OCS-Rust-Desktop-${desktopVersion}-macos-${runtime.arch}.zip`);
const scriptZip = join(out, `OCS-Rust-Userscript-${scriptVersion}.zip`);
const rawScript = join(out, 'ocs-rust.user.js');
// 只替换本工具生成的同名产物，不处理其他文件。
for (const path of [desktopZip, scriptZip]) await rm(path, { force: true });
command('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', app, desktopZip]);
const temporary = await mkdtemp(join(tmpdir(), 'ocs-release-'));
try {
	const folder = join(temporary, `OCS-Rust-Userscript-${scriptVersion}`);
	await mkdir(folder);
	await cp(script, join(folder, 'ocs-rust.user.js'));
	await cp(join(root, 'docs/userscript.md'), join(folder, 'README.md'));
	await cp(join(root, 'LICENSE'), join(folder, 'LICENSE'));
	await cp(join(root, 'assets/licenses/OCS-userscript.LICENSE'), join(folder, 'OCS-userscript.LICENSE'));
	command('ditto', ['-c', '-k', '--keepParent', folder, scriptZip]);
} finally {
	await rm(temporary, { recursive: true, force: true });
}
await cp(script, rawScript);
const files = await Promise.all([desktopZip, ...additionalFiles, scriptZip, rawScript].map(async (path) => ({
	name: basename(path), bytes: (await stat(path)).size, sha256: await sha256(path)
})));
const manifest = join(out, 'release-manifest.json');
await writeFile(manifest, JSON.stringify({
	project: 'OCS Rust',
	repository: 'https://github.com/mohui666/OCS-Rust',
	sourceCommit,
	desktopVersion, scriptVersion,
	upstream: { desktop: '2.12.0', desktopCommit: 'ecc6bb7ee79cb713caab7e896a913383e9437a14', userscript: '4.15.3' },
	target: { platform: runtime.platform, arch: runtime.arch, node: runtime.node, playwright: runtime.playwright },
	targets,
	createdAt: new Date().toISOString(),
	checks: { userscriptSyntax: 'passed', packageVersions: 'matched', bundledScript: 'matched', macOSSignatureIntegrity: 'passed', ...(additionalFiles.length ? { windowsSourceAndInstaller: 'matched' } : {}) },
	validation: 'Package checks cover versions, source provenance, bundled resources and hashes. Build, test and desktop verification results and untested real-model/course flows are documented in docs/verification.md.',
	files
}, null, 2) + '\n');
files.push({ name: basename(manifest), bytes: (await stat(manifest)).size, sha256: await sha256(manifest) });
await writeFile(join(out, 'SHA256SUMS.txt'), files.map(file => `${file.sha256}  ${file.name}`).join('\n') + '\n');
console.log(`桌面 ${desktopVersion} / 脚本 ${scriptVersion} 已分别打包：${out}`);
for (const file of files) console.log(`${file.name}\t${file.bytes} bytes`);
