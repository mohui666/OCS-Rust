<template>
  <a-button type="text" @click="visible = true">AI 题库</a-button>
  <a-modal v-model:visible="visible" title="AI 答题设置" :width="660" :footer="false" unmount-on-close :body-style="{ maxHeight: '74vh', overflowY: 'auto' }">
    <AISettings v-if="visible" />
  </a-modal>
</template>
<script setup lang="ts">
import { ref } from 'vue';
import AISettings from './AISettings.vue';
const visible = ref(false);
</script>
