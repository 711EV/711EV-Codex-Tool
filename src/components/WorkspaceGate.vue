<script setup lang="ts">
import { CircleAlert, Download, LoaderCircle, RefreshCw } from "lucide-vue-next";

defineProps<{
  state: "checking" | "missing" | "failed";
  error: string | null;
}>();
defineEmits<{ retry: []; download: [] }>();
</script>

<template>
  <div
    class="initialization-overlay"
    :class="{ 'workspace-gate': state !== 'checking' }"
    data-testid="workspace-gate"
    :data-state="state"
    :role="state === 'checking' ? 'status' : 'region'"
    :aria-label="state === 'checking' ? '初始化' : state === 'failed' ? '配置检测失败' : '未检测到 ChatGPT 客户端配置'"
  >
    <template v-if="state === 'checking'">
      <div class="initialization-loader" aria-hidden="true">
        <span class="initialization-loader-halo" />
        <span class="initialization-loader-ring" />
        <LoaderCircle class="initialization-loader-icon" :size="27" :stroke-width="1.8" />
      </div>
      <span class="initialization-label">初始化</span>
      <span class="initialization-dots" aria-hidden="true"><i /><i /><i /></span>
    </template>
    <template v-else>
      <CircleAlert class="workspace-gate-icon" :size="38" :stroke-width="1.5" aria-hidden="true" />
      <div class="workspace-gate-copy" role="status" aria-live="polite">
        <h2>{{ state === 'failed' ? '配置检测失败' : '未检测到 ChatGPT 客户端配置' }}</h2>
        <p v-if="state === 'missing'">请先安装并启动 ChatGPT 客户端，<br />再点击「重新检测」。</p>
        <p v-else class="workspace-gate-error">{{ error }}</p>
      </div>
      <div class="workspace-gate-actions">
        <button type="button" class="secondary-button" data-testid="retry-workspace" @click="$emit('retry')">
          <RefreshCw :size="14" aria-hidden="true" />重新检测
        </button>
        <button v-if="state === 'missing'" type="button" class="primary-button" data-testid="download-client" @click="$emit('download')">
          <Download :size="14" aria-hidden="true" />下载 ChatGPT
        </button>
      </div>
    </template>
  </div>
</template>
