<script setup lang="ts">
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Github, Minus, X } from "lucide-vue-next";
import titlebarLogoUrl from "../assets/app-icon-light.png";
import QQLogo from "./QQLogo.vue";

const desktopWindow = isTauri() ? getCurrentWindow() : null;
const githubUrl = "https://github.com/711EV/711EV-Codex-Tool";
const qqUrl = "https://qm.qq.com/q/e9xHZxgN4Q";

function shouldIgnoreDragTarget(target: EventTarget | null) {
  return target instanceof Element
    && Boolean(target.closest("button, a, input, select, textarea, [role='menu'], [data-no-drag]"));
}

function startWindowDrag(event: PointerEvent) {
  if (!desktopWindow || event.button !== 0 || shouldIgnoreDragTarget(event.target)) return;
  void desktopWindow.startDragging();
}

async function openExternal(url: string) {
  try {
    if (isTauri()) await openUrl(url);
    else window.open(url, "_blank", "noopener,noreferrer");
  } catch {
    // The main shell owns the Message queue; a failed link should not break
    // titlebar interactions or leave an unhandled promise in the WebView.
  }
}
</script>

<template>
  <header class="desktop-titlebar" @pointerdown="startWindowDrag">
    <div class="desktop-titlebar-brand" aria-label="ChatGPT中转工具">
      <img class="desktop-titlebar-logo" :src="titlebarLogoUrl" alt="" aria-hidden="true" />
      <span class="desktop-titlebar-name">ChatGPT中转工具</span>
    </div>
    <div class="desktop-window-controls" data-no-drag>
      <div class="desktop-link-controls">
        <button type="button" class="desktop-window-button desktop-link-button" aria-label="打开 GitHub 项目" @click="openExternal(githubUrl)">
          <Github :size="15" />
        </button>
        <button type="button" class="desktop-window-button desktop-link-button" aria-label="打开交流群" @click="openExternal(qqUrl)">
          <QQLogo :size="15" />
        </button>
      </div>
      <div class="desktop-action-controls">
        <button type="button" class="desktop-window-button" aria-label="最小化" @click="desktopWindow?.minimize()">
          <Minus :size="15" :stroke-width="1.8" aria-hidden="true" />
        </button>
        <button type="button" class="desktop-window-button desktop-window-button--close" aria-label="关闭到系统托盘" @click="desktopWindow?.close()">
          <X :size="14" :stroke-width="1.8" aria-hidden="true" />
        </button>
      </div>
    </div>
  </header>
</template>
