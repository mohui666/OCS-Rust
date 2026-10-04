<template>
  <a-config-provider :locale="zhCN"><main class="embedded-settings"><AISettings @changed="changed" /></main></a-config-provider>
</template>
<script setup lang="ts">
import AISettings from '../components/AISettings.vue';
import zhCN from '@arco-design/web-vue/es/locale/lang/zh-cn';
import { aiPageParams } from '../utils/ai-settings';
function changed() {
  const parent = aiPageParams().get('parent');
  if (parent) window.parent.postMessage({ type: 'ocs-ai-settings-changed' }, parent);
}
</script>
<style scoped>
.embedded-settings { padding: 4px 2px 12px; background: white; }
</style>
