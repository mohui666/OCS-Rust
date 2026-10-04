<template>
  <div class="ai-settings">
    <a-spin v-if="!data && !error" tip="正在读取 AI 设置…" />
    <a-alert v-if="error" type="error" class="message">{{ error }}</a-alert>
    <template v-if="data">
      <div class="selection-row">
        <div><strong>使用 AI 答题</strong><p>开启后，仅在搜题时调用所选模型。</p></div>
        <a-switch :model-value="selected" :disabled="busy" aria-label="使用 AI 答题" @change="selectAI" />
      </div>
      <div class="service-status">
        <span class="status-dot" :class="{ ready: running }"></span>
        <span>{{ processing ? '正在处理题目' : running ? '服务已就绪' : '服务尚未就绪' }}</span>
        <span v-if="running" class="muted">已解答 {{ data.status.completed }} · 处理中 {{ data.status.inflight }}</span>
      </div>
      <a-form :model="data.config" layout="vertical" :disabled="busy || processing">
        <a-form-item label="调用方式">
          <a-radio-group v-model="data.config.provider" type="button" @change="providerChanged">
            <a-radio value="codex">GPT 登录态</a-radio><a-radio value="api">API Key</a-radio>
          </a-radio-group>
          <template #extra>{{ data.config.provider === 'codex' ? '使用本机已登录的 Codex 账号，无需 API Key。' : '使用你填写的服务商 API。' }}</template>
        </a-form-item>
        <template v-if="data.config.provider === 'api'">
          <a-form-item label="API 地址"><a-input v-model="data.config.api_base_url" placeholder="https://api.example.com/v1" /></a-form-item>
          <a-form-item label="API Key">
            <a-input-password v-model="apiKey" autocomplete="new-password" :placeholder="data.api_key_configured ? '已保存，留空继续使用；输入新值可替换' : '输入服务商提供的 API Key'" />
            <template #extra>密钥仅加密保存在本机，不写入课程脚本或题库配置。</template>
          </a-form-item>
        </template>
        <a-form-item label="模型">
          <div class="model-row">
            <a-select v-model="model" :options="modelOptions" allow-search allow-create placeholder="选择或输入模型 ID" />
            <a-button :loading="loadingModels" :disabled="busy || processing" @click="loadModels">获取模型</a-button>
          </div>
          <template #extra>{{ modelNotice || '可以直接输入模型 ID；获取列表不会发起答题请求。' }}</template>
        </a-form-item>
        <a-form-item label="推理强度"><a-select v-model="reasoning" :options="reasoningOptions" /></a-form-item>
        <a-collapse class="advanced">
          <a-collapse-item key="advanced" header="高级设置">
            <template v-if="data.config.provider === 'api'">
              <a-form-item label="接口协议"><a-select v-model="data.config.api_protocol" :options="protocols" /></a-form-item>
              <a-form-item label="答案格式"><a-select v-model="data.config.api_response_format" :options="formats" /></a-form-item>
              <a-form-item label="温度（留空为服务商默认）"><a-input-number v-model="data.config.api_temperature" :min="0" :max="2" :step="0.1" allow-clear /></a-form-item>
              <a-form-item label="最大输出 Token（留空或 0 为默认）"><a-input-number v-model="data.config.api_max_output_tokens" :min="0" :precision="0" allow-clear /></a-form-item>
              <a-form-item v-if="data.config.api_protocol === 'chat_completions'" label="输出长度参数"><a-select v-model="data.config.api_token_parameter" :options="['max_completion_tokens','max_tokens']" /></a-form-item>
              <a-checkbox v-if="data.api_key_configured" v-model="clearKey">保存时删除已存 API Key</a-checkbox>
            </template>
            <template v-else>
              <a-button :disabled="busy" @click="detectPaths">自动配置路径</a-button>
              <p class="muted">{{ pathNotice || '默认自动识别本机程序和登录目录。' }}</p>
              <a-form-item label="Codex 程序"><a-input v-model="data.config.codex_bin" :readonly="!isNative" /></a-form-item>
              <a-form-item label="登录目录"><a-input v-model="data.config.codex_home" :readonly="!isNative" /></a-form-item>
            </template>
            <a-form-item label="超时秒数（0 为无限）"><a-input-number v-model="data.config.timeout_seconds" :min="0" /></a-form-item>
            <a-form-item label="并行页面数"><a-input-number v-model="data.config.max_concurrency" :min="1" :max="32" :precision="0" /></a-form-item>
            <a-form-item v-if="isNative" label="本地端口"><a-input-number v-model="data.config.port" :min="1024" :max="65535" :precision="0" /></a-form-item>
            <a-space v-if="isNative" wrap>
              <a-button :disabled="busy || processing" @click="importConfig">导入旧题库配置</a-button>
              <a-button :disabled="busy || !running" @click="copySubscription">复制订阅链接</a-button>
            </a-space>
          </a-collapse-item>
        </a-collapse>
      </a-form>
      <a-alert v-if="notice" type="success" class="message">{{ notice }}</a-alert>
      <a-alert v-if="data.status.last_error" type="warning" class="message">{{ data.status.last_error }}</a-alert>
      <div class="save-row">
        <span class="muted">打开面板、切换选项、保存设置均不会调用 AI 答题。</span>
        <a-button type="primary" :loading="busy && !loadingModels" :disabled="busy || processing" @click="save">保存设置</a-button>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue';
import { aiSettingsRequest } from '../utils/ai-settings';
import { isNative, invoke } from '../utils/native';

const emit = defineEmits<{ (e: 'changed'): void }>();
const data = ref<any>(), error = ref(''), notice = ref('');
const busy = ref(false), loadingModels = ref(false), apiKey = ref(''), clearKey = ref(false);
const models = ref<string[]>([]), modelNotice = ref(''), pathNotice = ref('');
const running = computed(() => data.value?.status?.status === 'running');
const processing = computed(() => running.value && data.value.status.inflight > 0);
const selected = computed(() => {
  const settings = data.value?.ocs?.store || {};
  const disabled = settings['common.settings.disabledAnswererWrapperNames'] || [];
  return !!data.value?.wrappers?.some((w: any) => !disabled.includes(w.name));
});
const model = computed({ get: () => data.value?.config?.[data.value.config.provider === 'api' ? 'api_model' : 'model'], set: value => { data.value.config[data.value.config.provider === 'api' ? 'api_model' : 'model'] = value; } });
const reasoning = computed({ get: () => data.value?.config?.[data.value.config.provider === 'api' ? 'api_reasoning_effort' : 'reasoning_effort'], set: value => { data.value.config[data.value.config.provider === 'api' ? 'api_reasoning_effort' : 'reasoning_effort'] = value; } });
const modelOptions = computed(() => [...new Set([model.value, ...models.value].filter(Boolean))]);
const reasoningOptions = computed(() => [{ label: '默认', value: '' }, ...['none','minimal','low','medium','high','xhigh'].map(value => ({ label: value, value }))]);
const protocols = [{ label: 'Chat Completions', value: 'chat_completions' }, { label: 'Responses', value: 'responses' }];
const formats = [{ label: 'JSON Object（兼容）', value: 'json_object' }, { label: 'JSON Schema（严格）', value: 'json_schema' }, { label: '仅提示词约束', value: 'prompt' }];

function submitted() { return { ...data.value.config, api_key: apiKey.value, clear_api_key: clearKey.value }; }
async function run(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true; error.value = ''; notice.value = '';
  try { await action(); } catch (e) { error.value = e instanceof Error ? e.message : String(e); }
  finally { busy.value = false; }
}
async function fetchModels() {
  loadingModels.value = true;
  try {
    const result = await aiSettingsRequest('models', submitted());
    models.value = result.models;
    modelNotice.value = `已获取 ${result.models.length} 个模型，也可手动输入 ID。`;
  } finally { loadingModels.value = false; }
}
async function loadModels() { await run(fetchModels); }
async function providerChanged() {
  models.value = []; modelNotice.value = ''; notice.value = '';
  if (data.value.config.provider === 'codex') await loadModels();
}
async function detectPaths() {
  await run(async () => {
    const paths = await aiSettingsRequest('detect_paths', submitted());
    for (const key of ['codex_bin', 'codex_home']) if (paths[key]) data.value.config[key] = paths[key];
    pathNotice.value = paths.codex_bin && paths.codex_home ? '已识别程序与登录目录。' : '未完整找到本机登录环境，请先在 Codex 登录。';
  });
}
async function selectAI(value: boolean | string | number) {
  await run(async () => {
    const current = await aiSettingsRequest('select', { selected: !!value });
    data.value.ocs = current.ocs;
    emit('changed');
    notice.value = value ? '已选用 AI，搜题时调用当前已保存的模型。' : '已关闭 AI 答题。';
  });
}
async function save() {
  await run(async () => {
    data.value = await aiSettingsRequest('save', submitted());
    apiKey.value = ''; clearKey.value = false;
    emit('changed');
    notice.value = '设置已保存，将用于下一次搜题。';
  });
}
async function importConfig() {
  await run(async () => {
    data.value = await aiSettingsRequest('import');
    apiKey.value = ''; clearKey.value = false;
    models.value = []; modelNotice.value = '';
    emit('changed');
    notice.value = '旧配置已导入。';
  });
}
async function copySubscription() {
  await run(async () => {
    await invoke('native_call', { op: 'clipboard.writeText', args: [data.value.subscription] });
    notice.value = '订阅链接已复制。';
  });
}
watch(() => data.value?.config?.api_base_url, () => { models.value = []; modelNotice.value = ''; });
let timer: ReturnType<typeof setInterval> | undefined, polling = false, disposed = false;
onMounted(async () => {
  await run(async () => {
    data.value = await aiSettingsRequest('get');
    if (data.value.config.provider === 'codex') {
      try { await fetchModels(); }
      catch { modelNotice.value = '未读到模型列表，可以手动输入 ID 或点击获取模型。'; }
    }
  });
  if (disposed) return;
  timer = setInterval(async () => {
    if (busy.value || polling || !data.value || document.hidden) return;
    polling = true;
    try {
      const current = await aiSettingsRequest('get');
      data.value.status = current.status;
      data.value.ocs = current.ocs;
    } catch { if (data.value) data.value.status = { status: 'stopped', last_error: '与 OCS 的连接已断开，请确认桌面程序正在运行。' }; }
    finally { polling = false; }
  }, 3000);
});
onBeforeUnmount(() => { disposed = true; if (timer) clearInterval(timer); apiKey.value = ''; });
</script>

<style scoped>
.ai-settings { color: #1d2129; font: 14px/1.5 -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
.selection-row { display: flex; align-items: center; justify-content: space-between; padding: 16px; border-radius: 10px; background: #f2f6ff; margin-bottom: 12px; }
.selection-row strong { font-size: 16px; }
.selection-row p { margin: 4px 0 0; color: #6b7280; }
.service-status { display: flex; gap: 8px; align-items: center; margin-bottom: 20px; font-size: 12px; }
.status-dot { width: 7px; height: 7px; border-radius: 50%; background: #ff7d00; }
.status-dot.ready { background: #00a870; }
.muted { color: #86909c; font-size: 12px; }
.model-row { display: flex; gap: 10px; width: 100%; }
.model-row .arco-select { min-width: 0; flex: 1; }
.advanced { margin: 6px 0 16px; }
.message { margin-bottom: 14px; }
.save-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 14px 0 0; border-top: 1px solid #e5e6eb; background: white; }
</style>
