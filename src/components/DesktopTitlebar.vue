<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Maximize2, Minimize2, Minus, X } from "lucide-vue-next";
import titlebarLogoUrl from "../assets/titlebar-logo-clear.png";

const desktopWindow = isTauri() ? getCurrentWindow() : null;
const isMaximized = ref(false);
let unlistenResize: (() => void) | undefined;

onMounted(async () => {
  if (!desktopWindow) return;
  await refreshMaximizedState();
  unlistenResize = await desktopWindow.onResized(refreshMaximizedState);
});

onBeforeUnmount(() => unlistenResize?.());

async function refreshMaximizedState() {
  if (!desktopWindow) return;
  isMaximized.value = await desktopWindow.isMaximized();
}

async function toggleMaximize() {
  if (!desktopWindow) return;
  await desktopWindow.toggleMaximize();
  await refreshMaximizedState();
}
</script>

<template>
  <header class="desktop-titlebar" data-tauri-drag-region @dblclick="toggleMaximize">
    <div class="desktop-titlebar-drag" data-tauri-drag-region @mousedown.left="desktopWindow?.startDragging()" />
    <div class="desktop-titlebar-brand" aria-label="711EV-Codex-Tool">
      <img class="desktop-titlebar-logo" :src="titlebarLogoUrl" alt="" aria-hidden="true" />
      <span class="desktop-titlebar-name">711EV-Codex-Tool</span>
    </div>
    <div class="desktop-window-controls">
      <button type="button" class="desktop-window-button" title="最小化" aria-label="最小化" @click="desktopWindow?.minimize()">
        <Minus :size="16" aria-hidden="true" />
      </button>
      <button
        type="button"
        class="desktop-window-button"
        :title="isMaximized ? '还原' : '最大化'"
        :aria-label="isMaximized ? '还原' : '最大化'"
        @click="toggleMaximize"
      >
        <Minimize2 v-if="isMaximized" :size="15" aria-hidden="true" />
        <Maximize2 v-else :size="15" aria-hidden="true" />
      </button>
      <button
        type="button"
        class="desktop-window-button desktop-window-button--close"
        title="关闭"
        aria-label="关闭"
        @click="desktopWindow?.close()"
      >
        <X :size="16" aria-hidden="true" />
      </button>
    </div>
  </header>
</template>
