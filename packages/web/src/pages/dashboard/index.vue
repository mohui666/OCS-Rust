<template>
	<div class="col-12 p-2 m-auto">
		<div
			class="text-secondary markdown mb-2"
			v-html="lang('notice_dashboard_monitor_page_usage', '')"
		></div>

		<div class="d-flex mb-1 align-items-center">
			<a-space :size="0">
				<template #split>
					<a-divider
						class="ms-1 me-1"
						direction="vertical"
					/>
				</template>

				<a-tooltip
					:content="
						launchedProcesses.length === 0
							? '暂无运行浏览器，无法开始监控'
							: '显示每个浏览器的图像，如果太多浏览器可能会造成电脑卡顿'
					"
					position="bl"
				>
					<div>
						<a-button
							size="mini"
							type="outline"
							:disabled="launchedProcesses.length === 0 || state.loading"
							@click="state.show = !state.show"
						>
							<template v-if="state.loading"> 加载中... </template>
							<template v-else> {{ state.show ? '暂停' : '开始' }}监控 </template>
						</a-button>
					</div>
				</a-tooltip>

				<a-switch v-model="store.render.dashboard.details.tags">
					<template #checked> 显示标签 </template>
					<template #unchecked> 显示标签 </template>
				</a-switch>

				<a-switch v-model="store.render.dashboard.details.notes">
					<template #checked> 显示备注 </template>
					<template #unchecked> 显示备注 </template>
				</a-switch>

				<a-select
					v-model="store.render.dashboard.num"
					size="mini"
					style="width: 96px"
					:options="[1, 2, 4, 6, 8].map((i) => ({ value: i, label: `显示${i}列` }))"
				>
				</a-select>

				<a-select
					v-model="store.render.dashboard.video.aspectRatio"
					size="mini"
					style="width: 130px"
					:options="aspectRatio"
				>
					<template #prefix> 横纵比 </template>
				</a-select>

				<a-select
					v-model="store.app.video_frame_rate"
					size="mini"
					style="width: 140px"
					:options="[
						{ label: '节能', value: 1 },
						{ label: '流畅', value: 4 },
						{ label: '高帧（消耗CPU）', value: 20 },
						{ label: '最高（很耗CPU）', value: 100 }
					]"
				>
					<template #prefix> 帧率 </template>
				</a-select>
			</a-space>
		</div>

		<template v-if="processes.length === 0">
			<div
				class="d-flex"
				style="height: 50vh"
			>
				<a-empty
					class="m-auto"
					description="没有运行中的浏览器"
				></a-empty>
			</div>
		</template>
		<template v-else-if="state.show === false">
			<a-empty
				class="pt-5"
				description="当前监控已暂停，请点击监控按钮重新监控"
			></a-empty>
		</template>
		<template v-else>
			<a-empty
				v-show="state.loading === true"
				class="pt-5"
				description="加载中..."
			></a-empty>
			<div
				v-show="state.loading === false"
				class="dashboard mt-2"
				:style="{
					'grid-template-columns': `repeat(${store.render.dashboard.num}, 1fr)`
				}"
			>
				<template
					v-for="pro of launchedProcesses"
					:key="pro.uid"
				>
					<div class="browser">
						<!-- 头部操作按钮 -->
						<div class="browser-title">
							<a-row
								style="overflow: overlay"
								class="flex-nowrap"
							>
								<a-col flex="auto">
									<span
										class="text-secondary"
										style="font-size: 12px"
									>
										{{ pro.browser.name }}
									</span>
								</a-col>
								<a-col
									flex="120px"
									class="d-flex align-content-center justify-content-end text-end"
								>
									<a-space
										:size="0"
										class="justify-content-end"
									>
										<template #split>
											<a-divider
												direction="vertical"
												class="ms-1 me-1"
											/>
										</template>

										<BrowserOperators
											:space="false"
											:browser="pro.browser"
										>
											<template #split>
												<a-divider
													direction="vertical"
													class="ms-1 me-1"
												/>
											</template>
										</BrowserOperators>

										<EntityOperator
											type="browser"
											:entity="pro.browser"
											:permissions="['location', 'edit']"
										></EntityOperator>
									</a-space>
								</a-col>
							</a-row>
						</div>

						<!-- 影像区域 -->
						<div
							class="browser-video"
							@click="openBrowser(pro.uid)"
						>
							<a-tooltip content="点击操控浏览器">
								<!-- 浏览器影像投屏占位符 -->
								<div :id="'video-' + pro.uid"></div>
							</a-tooltip>

							<span v-if="pro.video === undefined">
								<a-empty
									v-if="pro.status === 'launching'"
									description="等待浏览器启动..."
								>
								</a-empty>
								<a-empty
									v-else-if="pro.status === 'launched'"
									description="等待图像初始化..."
								>
								</a-empty>
							</span>
						</div>

						<!-- 显示浏览器信息 -->

						<a-row
							v-if="store.render.dashboard.details.notes || store.render.dashboard.details.tags"
							class="align-items-center"
						>
							<!-- 标签 -->
							<a-col
								v-if="store.render.dashboard.details.tags"
								style="width: 100px"
								flex="100px"
							>
								<Tags
									:tags="pro.browser.tags"
									:read-only="true"
									size="small"
								></Tags>
							</a-col>
							<!-- 备注 -->
							<a-col
								v-if="store.render.dashboard.details.notes"
								style="width: 100px"
								flex="100px"
								class="text-secondary notes"
							>
								<a-tooltip
									content="备注描述"
									position="tl"
								>
									<template #content>
										<div>备注描述</div>
										<a-divider class="mt-1 mb-1" />
										<div>
											{{ pro.browser.notes }}
										</div>
									</template>
									<span> {{ pro.browser.notes }} </span>
								</a-tooltip>
							</a-col>
						</a-row>
					</div>
				</template>
			</div>
		</template>
	</div>
</template>

<script setup lang="ts">
import { onDeactivated, watch, reactive, computed, onActivated, onMounted } from 'vue';
import { Process, processes } from '../../utils/process';
import BrowserOperators from '../../components/browsers/BrowserOperators.vue';
import { lang, store } from '../../store';
import Tags from '../../components/Tags.vue';
import { remote } from '../../utils/remote';
import { Modal, SelectOptionData } from '@arco-design/web-vue';
import EntityOperator from '../../components/EntityOperator.vue';
import type { DesktopCapturerSource } from 'electron';

const state = reactive({
	show: false,
	loading: false
});

const launchedProcesses = computed(() => processes.filter((p) => p.status === 'launched'));

const aspectRatio = [
	[0, '默认'],
	[4 / 3, '4:3'],
	[16 / 9, '16:9']
].map(
	(i) => ({ selected: i[0] === store.render.dashboard.video.aspectRatio, value: i[0], label: i[1] } as SelectOptionData)
);

// 监听已启动的浏览器，如果全部关闭，则关闭视频显示
watch(
	() => launchedProcesses.value.length,
	(curr, pre) => {
		// 如果全部关闭，则关闭视频显示
		if (launchedProcesses.value.length === 0) {
			// 触发 watch，关闭视频
			state.show = false;
		} else {
			// 如果有新的进程加入，则刷新视频
			if (state.show && curr > pre) {
				refreshVideo();
			}
		}
	}
);

watch(
	() => store.app.video_frame_rate,
	() => {
		Modal.info({
			content: '修改帧率后请 重启软件 才可生效。'
		});
	}
);

// 当横纵比改变时，延迟更新视频
watch(() => [store.render.dashboard.video.aspectRatio], refreshVideo);
// 当show改变时，即时更新
watch(
	() => state.show,
	() => {
		state.show ? refreshVideo() : closeVideo();
	}
);

/** 离开时视频可能会暂停，所以这里进行一下处理 */
onActivated(() => {
	for (const process of launchedProcesses.value) {
		process.video?.play();
	}
});

/** 离开时暂停视频播放 ，好像离开时原生事件也会自动暂停。 */
onDeactivated(() => {
	for (const process of launchedProcesses.value) {
		process.video?.pause();
	}
});

onMounted(() => {
	// 持续挂载视频，防止丢失
	setInterval(() => {
		for (const process of processes) {
			mountVideo(process);
		}
	}, 3000);
});

/**
 * 关闭视频显示
 */
const captures = new Map<string, () => void>();
async function closeVideo() {
 for (const stop of captures.values()) stop();
 captures.clear();
 for (const process of processes) { process.stream?.getTracks().forEach(track=>track.stop()); process.video=undefined; process.stream=undefined; }
}
async function refreshVideo() {
 state.loading=true;
 await closeVideo();
 try {
  for (const process of launchedProcesses.value) {
   await process.worker('gotoWebRTCPage');
   let source: any;
   try {
    for (let attempt=0;attempt<8&&!source;attempt++) {
     await new Promise(resolve=>setTimeout(resolve,250));
     const sources=await remote.methods.call('captureDesktopScreen');
     source=sources.find((s:any)=>String(s.name).includes(process.uid));
    }
    if(!source) throw new Error('无法识别窗口 '+process.browser.name+'，请检查系统录屏权限');
    const result=await getBrowserVideo(process.uid,[source]);
    process.video=result.video;process.stream=result.stream;
    mountVideo(process);
   } finally {await process.worker('closeWebRTCPage');}
  }
 } catch(e) {Modal.error({title:'监控画面获取失败',content:String(e)});} finally {state.loading=false;}
}
async function getBrowserVideo(uid:string,sources:any[]) {
 const source=sources[0]; const canvas=document.createElement('canvas');
 const ctx=canvas.getContext('2d')!; let stopped=false;let timer:ReturnType<typeof setTimeout>;
 const draw=async()=>{
  const url=await remote.methods.call('captureWindow',source.id);
  const img=new Image();img.src=url;await img.decode();
  if(stopped)return;
  if(canvas.width!==img.width||canvas.height!==img.height){canvas.width=img.width;canvas.height=img.height;}
  ctx.drawImage(img,0,0);
 };
 await draw();
 const fps=Math.min(15,Math.max(1,store.app.video_frame_rate||1));
 const stream=canvas.captureStream(fps);const video=document.createElement('video');
 video.srcObject=stream;video.muted=true;video.autoplay=true;video.playsInline=true;
 video.style.cssText='display:block;width:100%';
 const stop=()=>{stopped=true;clearTimeout(timer);stream.getTracks().forEach(t=>t.stop());};
 captures.set(uid,stop);
 const loop=async()=>{if(stopped||!stream.active){stop();return;}try{await draw();timer=setTimeout(loop,1000/fps);}catch(e){stop();Modal.error({title:'监控已停止',content:String(e)});}};
 timer=setTimeout(loop,1000/fps);
 return {video,stream};
}
//   挂载视频
function mountVideo(process: Process) {
	const slot = document.querySelector(`#video-${process.uid}`);
	// 如果 slot.children.length === 0 说明没有挂载视频
	if (slot && process.video && slot.firstElementChild !== process.video) {
		slot.replaceChildren(process.video);
	}
}

function openBrowser(uid: string) {
	Process.from(uid)?.bringToFront();
}
</script>

<style scoped lang="less">
.dashboard {
	display: grid;
	gap: 10px;
	grid-template-columns: repeat(6, 1fr);
}

.screenshot-item-title {
	padding: 0px 4px;
	white-space: nowrap;
}

.browser {
	background-color: #f2f5f8;
	padding: 4px;
	border-radius: 4px;

	&:hover {
		box-shadow: 0px 0px 4px -1px #2e98fc;
	}
}

.browser-video {
	overflow: hidden;
	cursor: pointer;
	border-radius: 4px;
}

.browser-title {
	height: 26px;
}

.browser-entity {
	padding: 4px;
}

.notes {
	font-size: 12px;
	text-overflow: ellipsis;
	white-space: nowrap;
	overflow: hidden;
}
</style>
