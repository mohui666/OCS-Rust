<template>
	<div class="title ps-2">
        <BridgePanel />
		<span
			class="logo"
			style="cursor: pointer; -webkit-app-region: no-drag"
			@click="shell.openExternal('https://docs.ocsjs.com')"
		>
			<img
				width="18"
				class="me-3"
				src="../../public/favicon.png"
			/>
		</span>
		<a-dropdown
			class="tittle-dropdown"
			trigger="hover"
			:popup-max-height="false"
		>
			<span class="title-item"> 工具 </span>
			<template #content>
				<a-doption style="width: 200px"> </a-doption>

				<a-doption @click="store.render.state.setup = true"> <Icon type="settings">初始化设置</Icon> </a-doption>
				<a-doption
					class="border-bottom"
					@click="checkBrowserCaches"
				>
					<Icon type="delete">清除浏览器缓存</Icon>
				</a-doption>

				<a-doption @click="migrateLegacy"> <Icon type="upload">迁移旧版资料（复制）</Icon> </a-doption>
                <a-doption @click="exportData"> <Icon type="save">导出数据</Icon> </a-doption>
				<a-doption
					class="border-bottom"
					@click="importData"
				>
					<Icon type="upload">导入数据</Icon>
				</a-doption>
				<a-doption @click="relaunch"> <Icon type="sync">重启软件</Icon> </a-doption>
				<a-doption @click="openLog"> <Icon type="folder">日志目录</Icon> </a-doption>
				<a-doption @click="openDevTools"> <Icon type="code">开发者工具</Icon> </a-doption>
			</template>
		</a-dropdown>

		<a-dropdown
			class="tittle-dropdown"
			position="bottom"
			trigger="hover"
			:popup-max-height="false"
		>
			<span class="title-item"> 帮助 </span>
			<template #content>
				<a-doption
					style="width: 200px"
					@click="about"
				>
					<Icon type="book">使用教程</Icon>
				</a-doption>
				<a-doption @click="allNotify"> <Icon type="notes">查看通知</Icon> </a-doption>

				<a-doption @click="showVersionLogs"> <Icon type="notes">更新日志</Icon> </a-doption>

				<TitleLink url="https://docs.ocsjs.com/">
					<template #title>
						<Icon type="home">软件官网</Icon>
					</template>
				</TitleLink>
			</template>
		</a-dropdown>

		<span
			class="title-item mode-switch"
			@click="toggleMode"
		>
			<Icon
				:type="isSimpleMode ? 'arrow_back' : 'arrow_forward'"
				style="font-size: 14px; vertical-align: middle"
			></Icon>
			{{ isSimpleMode ? '专业模式' : '简洁模式' }}
		</span>

		<StatusBar />
	</div>
</template>

<script setup lang="ts">
import BridgePanel from './BridgePanel.vue';
import { computed, h, ref } from 'vue';
import { fetchRemoteNotify, date, about, getRemoteInfos } from '../utils';
import { remote } from '../utils/remote';
import TitleLink from './TitleLink.vue';
import { Message, Modal, Input } from '@arco-design/web-vue';
import { store } from '../store/index';
import { router } from '../route';
import { electron } from '../utils/node';
import { currentBrowser, currentFolder, currentEntities, currentSearchedEntities } from '../fs/index';
import { Folder, root } from '../fs/folder';
import { FolderOptions, FolderType } from '../fs/interface';
import { Browser } from '../fs/browser';
import { checkBrowserCaches } from '../utils/browser';
import Icon from './Icon.vue';
import StatusBar from './StatusBar.vue';

const { shell } = electron;

const isSimpleMode = computed(() => router.currentRoute.value.path === '/simple');

/** 切换模式 */
function toggleMode() {
	if (isSimpleMode.value) {
		store.render.setting.mode = 'professional';
		router.push('/browsers');
	} else {
		store.render.setting.mode = 'simple';
		router.push('/simple');
	}
}

// 重启
function relaunch() {
	remote.app.call('relaunch');
}

// 打开日志目录
async function openLog() {
	const path = await remote.app.call('getPath', 'logs');
	shell.openPath(path);
}

// 显示全部通知
function allNotify() {
	fetchRemoteNotify(true);
}

async function migrateLegacy() {
 try {
  const result=await remote.methods.call('importLegacy');
  Object.assign(store,result.store);
  Modal.success({title:'迁移完成',content:'已复制 '+result.report.browsers+' 个浏览器的资料。旧版配置和文件保持原样。',onOk:()=>remote.app.call('relaunch')});
 } catch(error){Message.error(String(error));}
}

function importData() {
	remote.dialog
		.call('showOpenDialog', {
			title: '选择导入的数据文件',
			buttonLabel: '导入',
			filters: [{ extensions: ['ocsdata', 'json'], name: 'ocsdata' }]
		})
		.then(async ({ canceled, filePaths }) => {
			if (canceled === false && filePaths.length) {
				try {
					const text = await remote.fs.call('readFileSync', filePaths[0], { encoding: 'utf8' });
					const _store: typeof store = JSON.parse(text.toString());

					// 如果 render 是加密字符串，先解密为明文再导入
					if (typeof _store.render === 'string') {
						const renderStr = _store.render as string;
						let password: string | null = '';
                        if (renderStr.startsWith('export1:')) {password=await exportPassword(false);if(password===null)return;}
						const data = JSON.parse(await remote.methods.call(renderStr.startsWith('export1:')?'decryptExport':'decryptRenderString', renderStr, password) as string);
						(_store as any).render = data;
					}

					const root = _store.render.browser.root;
					// 遍历文件夹，将每个浏览器的缓存路径解析为用户数据目录下的文件夹
					const folders: FolderOptions<any, Folder<FolderType> | Browser>[] = [root];
					while (folders.length) {
						const folder = folders.shift();
						if (!folder) continue;
						if (Object.keys(folder.children || {}).length) {
							for (const key in folder.children) {
								if (Object.prototype.hasOwnProperty.call(folder.children, key)) {
									const entity = folder.children[key];
									if (!entity) continue;
									if (entity.type === 'folder') {
										folders.push(entity as any);
									} else if (entity.type === 'browser') {
										(entity as any).cachePath = await remote.path.call(
											'join',
											store.paths.userDataDirsFolder,
											entity.uid
										);
									}
								}
							}
						}
					}
					// 导入 store render 数据
					store.render = _store.render;

					Modal.success({
						title: '导入成功',
						content: () =>
							h('div', [
								'数据重启软件后生效。',
								'此入口导入配置并使用独立的浏览器目录。复制旧版登录资料请使用“迁移旧版资料（复制）”。'
							]),
						okText: '重启软件',
						cancelText: '稍后重启',
						hideCancel: false,
						simple: false,
						onOk() {
							remote.app.call('relaunch');
						}
					});
				} catch (err) {
					Message.error('数据有误! : ' + err);
				}
			}
		});
}
function exportData() {
	Modal.confirm({
		title: '导出数据',
		content: '数据中包含自动化程序的配置（例如账号密码），请小心保存防止泄露。导出后可在其他电脑中恢复数据。',
		okText: '确认',
		cancelText: '取消',
		onOk() {
			remote.dialog
				.call('showSaveDialog', {
					title: '选择导出位置',
					buttonLabel: '导出',
					defaultPath: `OCS软件数据导出_-${date(Date.now())}`
				})
				.then(async ({ canceled, filePath }) => {
					if (canceled === false && filePath) {
                        const password = await exportPassword(true);
                        if (password === null) return;
						const _store: typeof store = JSON.parse(JSON.stringify(store));

						const root = _store.render.browser.root;
						// 遍历文件夹，将每个浏览器的缓存路径改成相对路径
						const folders: FolderOptions<any, Folder<FolderType> | Browser>[] = [root];
						while (folders.length) {
							const folder = folders.shift();
							if (!folder) continue;
							if (Object.keys(folder.children || {}).length) {
								for (const key in folder.children) {
									if (Object.prototype.hasOwnProperty.call(folder.children, key)) {
										const entity = folder.children[key];
										if (!entity) continue;
										if (entity.type === 'folder') {
											folders.push(entity as any);
										} else if (entity.type === 'browser') {
											(entity as any).cachePath = '$CACHE_PATH';
										}
									}
								}
							}
						}

						// 导出前加密 render 数据，防止明文泄露
						if (typeof _store.render !== 'string') {
							(_store as any).render = await remote.methods.call(
								'encryptExport',
								JSON.stringify(_store.render), password
							) as string;
						}

						// 删除多余数据
						const filter_keys: (keyof typeof _store)[] = ['paths', 'app', 'window', 'server'];
						for (const key of filter_keys) {
							delete _store[key];
						}

						await remote.fs.call('writeFileSync', filePath + '.ocsdata', JSON.stringify(_store, null, 4));
						Message.success('导出成功！');
					}
				}).catch(error=>Message.error('导出失败：'+String(error)));
		}
	});
}
function exportPassword(creating: boolean): Promise<string | null> {
    const password=ref(''), repeat=ref('');
    return new Promise(resolve=>Modal.confirm({
        title: creating?'设置导出密码':'输入导出密码',
        content:()=>h('div',[
            h('p',creating?'此密码用于在其他电脑导入资料，请妥善保存。':'使用导出时设置的密码解密资料。'),
            h(Input.Password,{modelValue:password.value,'onUpdate:modelValue':(v:string)=>password.value=v,placeholder:'导出密码'}),
            ...(creating?[h(Input.Password,{modelValue:repeat.value,'onUpdate:modelValue':(v:string)=>repeat.value=v,placeholder:'再次输入密码',style:'margin-top:12px'})]:[])
        ]),
        onBeforeOk:()=>{
            if(creating && (Array.from(password.value).length<8 || password.value!==repeat.value)){Message.error('密码至少 8 个字符，且两次输入一致');return false;}
            if(!password.value){Message.error('请输入密码');return false;}
            resolve(password.value);return true;
        },
        onCancel:()=>resolve(null)
    }));
}

function openDevTools() {
	// @ts-ignore
	window.ocs = {
		currentBrowser,
		currentEntities,
		currentFolder,
		currentSearchedEntities,
		root,
		store
	};

	remote.webContents.call('openDevTools');
}

async function showVersionLogs() {
	const infos = await getRemoteInfos();

	Modal.confirm({
		title: () => '🎉 更新日志 🎉',
		okText: '确定',
		hideCancel: true,
		simple: true,
		width: 600,
		content: () =>
			h('div', [
				h('h', [
					'可前往官网下载最新版本：',
					h(
						'a',
						{
							href: 'https://docs.ocsjs.com/docs/app',
							target: '_blank'
						},
						'https://docs.ocsjs.com/docs/app'
					)
				]),
				h(
					'div',
					{
						style: {
							maxHeight: '320px',
							overflow: 'auto'
						}
					},
					infos.versions.map((item) =>
						h('div', [
							h(
								'div',
								{
									style: {
										marginBottom: '6px',
										fontWeight: 'bold'
									}
								},
								item.tag
							),
							h(
								'ul',
								(item.description.feat || [])
									.concat(item.description.fix || [])
									.concat(item.description.other || [])
									.map((text: string) => h('li', text))
							)
						])
					)
				)
			])
	});
}
</script>

<style scoped lang="less">
.title {
	-webkit-app-region: drag;
	width: 100%;
	display: flex;
	align-items: center;
	/** 系统自带控件高度为 32 */
	height: var(--title-height);
	cursor: default;
	border-bottom: 1px solid #f3f3f3;

	z-index: 999999;
	position: relative;
	background-color: white;

	.title-item {
		-webkit-app-region: no-drag;
		padding: 0px 8px;
		font-size: 14px;
		cursor: pointer;

		&:hover {
			background-color: #f0f0f0;
		}
	}

	.mode-switch {
		color: #86909c;
		font-size: 12px;
		display: inline-flex;
		align-items: center;
		gap: 2px;

		&:hover {
			color: #165dff;
		}
	}

	> span {
		display: flex;
	}
}
:deep(.ant-dropdown-menu-item) {
	font-size: 12px;
	padding: 2px 24px 2px 12px;
}

:deep(.arco-dropdown-option-content) {
	width: 100%;
	display: block;
}

.tutorial-tooltip {
	padding: 8px 12px 0px 12px !important;
}

body.platform-darwin {
	.title {
		justify-content: center;
	}
}
</style>
