<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Check,
  ChevronsUpDown,
  Database,
  Github,
  BookOpen,
  Rocket,
  Zap,
} from "lucide-vue-next";
import type { Profile } from "../types";
import QQLogo from "./QQLogo.vue";

const props = defineProps<{
  codexHome?: string | null;
  profiles?: Profile[];
  activeProfileId?: string | null;
}>();

const emit = defineEmits<{
  error: [message: string];
  "select-profile": [profileId: string];
}>();

const storageRoot = ref<HTMLElement | null>(null);
const storageMenuOpen = ref(false);
const hasMultipleStorageLocations = computed(() => (props.profiles?.length ?? 0) > 1);
const githubRepositoryUrl = "https://github.com/711EV/711EV-Codex-Tool";

const menuLinks = [
  {
    label: "推荐梯子",
    url: "https://www.tntv2.net/auth/register?code=oow59s",
    icon: Rocket,
  },
  {
    label: "711EV中转站",
    url: "https://ai.711ev.com/",
    icon: Zap,
  },
  {
    label: "使用教程",
    url: "https://docs.711ev.com/#/711ev-relay/guide/codex-tool",
    icon: BookOpen,
  },
  {
    label: "交流群",
    url: "https://qm.qq.com/q/e9xHZxgN4Q",
    icon: QQLogo,
  },
] as const;

async function openExternalLink(label: string, url: string) {
  try {
    if (isTauri()) {
      await openUrl(url);
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  } catch (reason) {
    const detail = reason instanceof Error ? reason.message : String(reason);
    emit("error", `无法打开${label}：${detail}`);
  }
}

function toggleStorageMenu() {
  if (!hasMultipleStorageLocations.value) return;
  storageMenuOpen.value = !storageMenuOpen.value;
}

function selectStorageLocation(profileId: string) {
  storageMenuOpen.value = false;
  if (profileId !== props.activeProfileId) emit("select-profile", profileId);
}

function closeStorageMenuFromOutside(event: PointerEvent) {
  if (!storageRoot.value?.contains(event.target as Node)) storageMenuOpen.value = false;
}

function closeStorageMenuFromKeyboard(event: KeyboardEvent) {
  if (event.key === "Escape") storageMenuOpen.value = false;
}

onMounted(() => {
  document.addEventListener("pointerdown", closeStorageMenuFromOutside);
  document.addEventListener("keydown", closeStorageMenuFromKeyboard);
});

onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", closeStorageMenuFromOutside);
  document.removeEventListener("keydown", closeStorageMenuFromKeyboard);
});
</script>

<template>
  <nav class="top-menu" aria-label="快捷菜单">
    <div ref="storageRoot" class="top-menu-storage-cluster">
      <div
        class="top-menu-storage"
        :title="codexHome ?? '未发现存储位置'"
        :aria-label="`存储位置：${codexHome ?? '未发现'}`"
      >
        <Database :size="15" aria-hidden="true" />
        <span class="top-menu-storage-label">存储位置</span>
        <strong class="top-menu-storage-path">{{ codexHome ?? "未发现" }}</strong>
      </div>

      <button
        type="button"
        class="top-menu-storage-switch"
        data-testid="storage-location-switch"
        :title="hasMultipleStorageLocations ? '切换存储位置' : '暂无其他存储位置'"
        aria-label="切换存储位置"
        :aria-expanded="storageMenuOpen && hasMultipleStorageLocations"
        aria-haspopup="menu"
        :disabled="!hasMultipleStorageLocations"
        @click="toggleStorageMenu"
      >
        <ChevronsUpDown :size="14" aria-hidden="true" />
      </button>

      <div
        v-if="storageMenuOpen && hasMultipleStorageLocations"
        class="storage-location-menu"
        role="menu"
      >
        <button
          v-for="profile in profiles"
          :key="profile.id"
          type="button"
          class="storage-location-option"
          :class="{ active: profile.id === activeProfileId }"
          :title="profile.codexHome"
          role="menuitemradio"
          :aria-checked="profile.id === activeProfileId"
          @click="selectStorageLocation(profile.id)"
        >
          <Database :size="15" aria-hidden="true" />
          <span class="storage-location-copy">
            <strong>{{ profile.name }}</strong>
            <small>{{ profile.codexHome }}</small>
          </span>
          <Check
            v-if="profile.id === activeProfileId"
            class="storage-location-check"
            :size="15"
            aria-hidden="true"
          />
        </button>
      </div>
    </div>

    <div class="top-menu-links">
      <button
        v-for="item in menuLinks"
        :key="item.url"
        type="button"
        class="top-menu-link top-menu-resource-link"
        :title="`在浏览器中打开${item.label}`"
        @click="openExternalLink(item.label, item.url)"
      >
        <component :is="item.icon" :size="15" aria-hidden="true" />
        <span>{{ item.label }}</span>
      </button>
      <button
        type="button"
        class="top-menu-link top-menu-github-link"
        data-testid="github-project-link"
        title="打开 GitHub 项目"
        aria-label="打开 GitHub 项目"
        @click="openExternalLink('GitHub 项目', githubRepositoryUrl)"
      >
        <Github :size="17" aria-hidden="true" />
      </button>
    </div>
  </nav>
</template>
