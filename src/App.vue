<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Archive, Check, ChevronDown, ChevronsUpDown, CircleAlert, CircleCheck, Copy, Database, Download, Edit3, Eye, EyeOff, Folder, History, Info, LoaderCircle, Plus, RefreshCw, Rocket, Server, Trash2, TriangleAlert, X } from "lucide-vue-next";
import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import DesktopTitlebar from "./components/DesktopTitlebar.vue";
import AppTooltip from "./components/AppTooltip.vue";
import ModalDialog from "./components/ModalDialog.vue";
import OpenAILogo from "./components/OpenAILogo.vue";
import { appUpdater } from "./services/appUpdater";
import { backend } from "./services/backend";
import { useWorkspaceStore } from "./stores/workspace";
import { useMessageStore, type MessageTone } from "./stores/messages";
import type { ApplicationUpdate, ProviderConfigInput, ProviderSessionRecord, ProviderBucket } from "./types";

const workspace = useWorkspaceStore();
const messages = useMessageStore();
const appVersion = __APP_VERSION__;
const configDialog = ref<"add" | "edit" | null>(null);
const configDraft = ref<ProviderConfigInput>({ profileId: "", providerId: "", baseUrl: "", apiKey: "", template: "other" });
const configUse711Ev = ref(false);
const configSaving = ref(false);
const configKeyVisible = ref(false);
const recoveryOpen = ref(false);
const recoveryLoading = ref(false);
const recoveryProviderId = ref<string | null>(null);
const recoveryGroups = ref<Array<{ provider: string; sessions: ProviderSessionRecord[] }>>([]);
const recoveryCollapsedProviders = ref<Record<string, boolean>>({});
const recoverySelected = ref<string[]>([]);
const cleanupOpen = ref(false);
const cleanupLoading = ref(false);
const cleanupProviderId = ref<string | null>(null);
const cleanupSessions = ref<ProviderSessionRecord[]>([]);
const cleanupSelected = ref<string[]>([]);
const forcePrompt = ref(false);
const pendingOperation = ref<(() => Promise<void>) | null>(null);
const pendingOperationLabel = ref("当前操作");
const operationBusy = ref(false);
const switchPrompt = ref<string | null>(null);
const switchRestarting = ref(false);
const updateInfo = ref<ApplicationUpdate | null>(null);
const updateOpen = ref(false);
const updateChecking = ref(false);
const updateInstalling = ref(false);
const providerConfigs = ref<Record<string, { baseUrl: string | null; apiKeyMasked: string | null; authStatus: string | null; configured: boolean }>>({});
const providerApiKeys = ref<Record<string, string>>({});
const providerSizes = ref<Record<string, number>>({});
const providerRefreshing = ref<Record<string, boolean>>({});
const providersRefreshing = ref(false);
const storageRoot = ref<HTMLElement | null>(null);
const storageMenuOpen = ref(false);
const hasMultipleStorageLocations = computed(() => workspace.profiles.length > 1);
const initializingApp = ref(true);
let unlistenTrayUpdate: UnlistenFn | null = null;

function handleKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  if (storageMenuOpen.value) {
    storageMenuOpen.value = false;
    event.preventDefault();
    return;
  }
  if (forcePrompt.value) {
    cancelForcePrompt();
  } else if (switchPrompt.value && !switchRestarting.value) {
    switchPrompt.value = null;
  } else if (updateOpen.value && !updateInstalling.value) {
    updateOpen.value = false;
  } else if (cleanupOpen.value && !operationBusy.value) {
    cleanupOpen.value = false;
  } else if (recoveryOpen.value && !operationBusy.value) {
    recoveryOpen.value = false;
  } else if (configDialog.value && !configSaving.value) {
    configDialog.value = null;
  } else {
    return;
  }
  event.preventDefault();
}

function toggleStorageMenu() {
  if (!hasMultipleStorageLocations.value) return;
  storageMenuOpen.value = !storageMenuOpen.value;
}
function selectStorageLocation(profileId: string) {
  storageMenuOpen.value = false;
  if (profileId !== workspace.activeProfileId) void selectProfile(profileId);
}
function closeStorageMenuFromOutside(event: PointerEvent) {
  if (!storageRoot.value?.contains(event.target as Node)) storageMenuOpen.value = false;
}

function notify(content: string, tone: MessageTone = "neutral", options?: { duration?: number; key?: string }) {
  messages.showMessage(content, tone, options);
}
function isOfficial(providerId: string) { return providerId.toLowerCase() === "openai"; }
function providerDisplayName(providerId: string) { return isOfficial(providerId) ? "官方账号" : providerId; }
function authStatusLabel(status: string | null | undefined, configured?: boolean) {
  if (configured === false) return "未配置";
  if (!status || status === "missing") return "未认证";
  if (["available", "captured", "ok", "active"].includes(status.toLowerCase())) return "已认证";
  if (["conflict", "stale"].includes(status.toLowerCase())) return "需重新认证";
  return status;
}
function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}
function providerApiKeyValue(providerId: string) {
  return providerApiKeys.value[providerId] ?? "未配置";
}
function providerBaseUrlValue(providerId: string) {
  const config = providerConfigs.value[providerId];
  return config?.configured ? config.baseUrl ?? "未配置" : "未配置";
}
async function copyTextValue(label: string, value: string | null | undefined) {
  const text = value?.trim();
  if (!text || text === "未配置") {
    notify(`${label}暂无可复制内容`, "warning", { key: "provider-copy" });
    return;
  }
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
    } else {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.left = "-9999px";
      document.body.appendChild(textarea);
      try {
        textarea.select();
        if (!document.execCommand("copy")) throw new Error("当前环境不支持剪贴板写入");
      } finally {
        textarea.remove();
      }
    }
    notify(`${label}已复制`, "success", { key: "provider-copy" });
  } catch (error) {
    notify(`${label}复制失败：${error instanceof Error ? error.message : String(error)}`, "danger", { key: "provider-copy" });
  }
}
function providerTotalSize(providerId: string) {
  return providerSizes.value[providerId] ?? recoveryGroups.value.find((group) => group.provider === providerId)?.sessions.reduce((sum, item) => sum + item.sizeBytes, 0) ?? 0;
}
function statusLabel(session: ProviderSessionRecord) {
  if (session.archived) return "已归档";
  if (session.sourceKind === "internal" || session.parentThreadId) return "子会话";
  return "主会话";
}
function toggleRecoverySession(session: ProviderSessionRecord, provider: string) {
  if (provider.toLowerCase() === recoveryProviderId.value?.toLowerCase() || statusLabel(session) !== "主会话") return;
  recoverySelected.value = recoverySelected.value.includes(session.threadId)
    ? recoverySelected.value.filter((id) => id !== session.threadId)
    : [...recoverySelected.value, session.threadId];
}
function isRecoveryGroupCollapsed(provider: string) {
  return recoveryCollapsedProviders.value[provider] === true;
}
function toggleRecoveryGroup(provider: string) {
  recoveryCollapsedProviders.value = {
    ...recoveryCollapsedProviders.value,
    [provider]: !isRecoveryGroupCollapsed(provider),
  };
}
function toggleCleanupSession(session: ProviderSessionRecord) {
  if (operationBusy.value) return;
  cleanupSelected.value = cleanupSelected.value.includes(session.threadId)
    ? cleanupSelected.value.filter((id) => id !== session.threadId)
    : [...cleanupSelected.value, session.threadId];
}

async function openExternal(url: string) {
  try {
    if (isTauri()) await openUrl(url); else window.open(url, "_blank", "noopener,noreferrer");
  } catch (error) { notify(`无法打开链接：${error instanceof Error ? error.message : String(error)}`, "danger"); }
}
async function refreshProviders(showMessage = true) {
  if (providersRefreshing.value) return;
  providersRefreshing.value = true;
  try {
    await workspace.refreshProviders();
    const profileId = workspace.activeProfileId;
    if (profileId) {
      const entries = await Promise.all(workspace.providerBuckets.map(async (provider) => {
        try {
          const [config, snapshot] = await Promise.all([
            backend.providerConfigRead(profileId, provider.providerId),
            backend.providerWorkspace(profileId, provider.providerId),
          ]);
          const apiKey = !isOfficial(provider.providerId) && config.configured && config.apiKeyMasked
            ? await backend.providerConfigRevealKey(profileId, provider.providerId).catch(() => null)
            : null;
          return [provider.providerId, { config: { baseUrl: config.baseUrl, apiKeyMasked: config.apiKeyMasked, authStatus: config.officialAuthSnapshotStatus, configured: config.configured }, apiKey, size: snapshot.providerSessions.reduce((sum, session) => sum + session.sizeBytes, 0) }] as const;
        } catch { return [provider.providerId, { config: { baseUrl: null, apiKeyMasked: null, authStatus: null, configured: false }, apiKey: null, size: 0 }] as const; }
      }));
      providerConfigs.value = Object.fromEntries(entries.map(([id, value]) => [id, value.config]));
      providerSizes.value = Object.fromEntries(entries.map(([id, value]) => [id, value.size]));
      providerApiKeys.value = Object.fromEntries(entries.flatMap(([id, value]) => value.apiKey ? [[id, value.apiKey]] : []));
    } else {
      providerConfigs.value = {};
      providerSizes.value = {};
      providerApiKeys.value = {};
    }
    if (showMessage && !workspace.error) notify("供应商列表已刷新", "success");
  } finally {
    providersRefreshing.value = false;
  }
}
async function refreshProvider(provider: ProviderBucket) {
  const providerId = provider.providerId;
  if (providerRefreshing.value[providerId]) return;
  const profileId = provider.profileId || workspace.activeProfileId;
  if (!profileId) {
    notify("未选择配置目录", "warning");
    return;
  }
  providerRefreshing.value = { ...providerRefreshing.value, [providerId]: true };
  try {
    const [config, snapshot] = await Promise.all([
      backend.providerConfigRead(profileId, providerId),
      backend.providerWorkspace(profileId, providerId),
    ]);
    const apiKey = !isOfficial(providerId) && config.configured && config.apiKeyMasked
      ? await backend.providerConfigRevealKey(profileId, providerId).catch(() => null)
      : null;
    const freshBucket = snapshot.providerBuckets.find((item) => item.providerId === providerId);
    const bucketIndex = workspace.providerBuckets.findIndex((item) => item.providerId === providerId);
    if (freshBucket && bucketIndex >= 0) {
      workspace.providerBuckets[bucketIndex] = { ...workspace.providerBuckets[bucketIndex], ...freshBucket };
    }
    providerConfigs.value = {
      ...providerConfigs.value,
      [providerId]: {
        baseUrl: config.baseUrl,
        apiKeyMasked: config.apiKeyMasked,
        authStatus: config.officialAuthSnapshotStatus,
        configured: config.configured,
      },
    };
    const nextApiKeys = { ...providerApiKeys.value };
    if (apiKey) nextApiKeys[providerId] = apiKey;
    else delete nextApiKeys[providerId];
    providerApiKeys.value = nextApiKeys;
    providerSizes.value = {
      ...providerSizes.value,
      [providerId]: snapshot.providerSessions.reduce((sum, session) => sum + session.sizeBytes, 0),
    };
    notify(`${providerDisplayName(providerId)} 信息已刷新`, "success");
  } catch (error) {
    notify(error instanceof Error ? error.message : String(error), "danger");
  } finally {
    const next = { ...providerRefreshing.value };
    delete next[providerId];
    providerRefreshing.value = next;
  }
}
function providerUseState(provider: ProviderBucket) {
  if (provider.isCurrent) return "current";
  if (provider.configured === false) return "unconfigured";
  return "available";
}
async function selectProfile(profileId: string) {
  await workspace.selectProfile(profileId);
  await refreshProviders(false);
}
async function openConfig(mode: "add" | "edit", provider?: ProviderBucket) {
  if (mode === "edit" && (!provider || isOfficial(provider.providerId))) { notify("官方供应商不支持修改", "warning"); return; }
  if (mode === "edit" && provider) {
    const config = await backend.providerConfigRead(provider.profileId, provider.providerId);
    configDraft.value = { profileId: config.profileId, providerId: config.providerId, baseUrl: config.baseUrl ?? "", apiKey: "", template: config.providerId === "711EV" ? "711ev" : "other" };
    if (config.managedByTool && config.apiKeyMasked) {
      try { configDraft.value.apiKey = await backend.providerConfigRevealKey(config.profileId, config.providerId); } catch { /* key stays empty */ }
    }
    configUse711Ev.value = config.providerId === "711EV";
  } else {
    configDraft.value = { profileId: workspace.activeProfileId ?? "", providerId: "", baseUrl: "", apiKey: "", template: "other" };
    configUse711Ev.value = false;
  }
  configKeyVisible.value = false;
  configDialog.value = mode;
}
function toggle711EvPreset() {
  configUse711Ev.value = !configUse711Ev.value;
  configDraft.value.template = configUse711Ev.value ? "711ev" : "other";
  if (configUse711Ev.value) {
    configDraft.value.providerId = "711EV";
    configDraft.value.baseUrl = "https://ai.711ev.com/v1";
  } else {
    configDraft.value.providerId = "";
    configDraft.value.baseUrl = "";
    configDraft.value.apiKey = "";
  }
}
function closeConfig() { if (!configSaving.value) configDialog.value = null; }
async function saveConfig() {
  if (!configDraft.value.profileId || !configDraft.value.providerId.trim()) { notify("请填写供应商名称", "warning"); return; }
  configSaving.value = true;
  try {
    await workspace.saveProviderConfig({ ...configDraft.value, providerId: configDraft.value.providerId.trim(), template: configUse711Ev.value ? "711ev" : "other" });
    configDialog.value = null;
    configUse711Ev.value = false;
    await refreshProviders(false);
    notify("供应商配置已保存", "success");
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { configSaving.value = false; }
}
async function useProvider(provider: ProviderBucket) {
  if (provider.isCurrent || provider.configured === false || workspace.providerConfigSwitching) return;
  try {
    await workspace.selectProvider(provider.providerId);
    await workspace.switchProvider(provider.providerId);
    switchPrompt.value = provider.providerId;
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
}
async function restartAfterSwitch() {
  if (!switchPrompt.value || switchRestarting.value || !workspace.activeProfileId) return;
  switchRestarting.value = true;
  try {
    await backend.restartCodexClient(workspace.activeProfileId);
    notify("ChatGPT 已重启，并会读取刚刚写入的供应商配置", "success");
    switchPrompt.value = null;
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { switchRestarting.value = false; }
}
async function openRecovery(provider: ProviderBucket) {
  recoveryProviderId.value = provider.providerId;
  recoverySelected.value = [];
  recoveryCollapsedProviders.value = {};
  recoveryLoading.value = true;
  recoveryOpen.value = true;
  try {
    const ids = [...new Set(workspace.providerBuckets
      .map((item) => item.providerId)
      .filter((id) => id.toLowerCase() !== provider.providerId.toLowerCase()))];
    recoveryGroups.value = (await Promise.all(ids.map(async (id) => ({
      provider: id,
      sessions: (await backend.providerWorkspace(provider.profileId, id)).providerSessions
        .filter((session) => statusLabel(session) === "主会话"),
    })))).filter((group) => group.sessions.length > 0);
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { recoveryLoading.value = false; }
}
function closeRecovery() { if (!operationBusy.value) recoveryOpen.value = false; }
async function openCleanup(provider: ProviderBucket) {
  cleanupProviderId.value = provider.providerId;
  cleanupSelected.value = [];
  cleanupSessions.value = [];
  cleanupLoading.value = true;
  cleanupOpen.value = true;
  try {
    cleanupSessions.value = (await backend.providerWorkspace(provider.profileId, provider.providerId)).providerSessions
      .filter((session) => statusLabel(session) === "已归档" || statusLabel(session) === "子会话");
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { cleanupLoading.value = false; }
}
function closeCleanup() { if (!operationBusy.value) cleanupOpen.value = false; }
function requestCloseAndRun(label: string, operation: () => Promise<void>) {
  pendingOperationLabel.value = label;
  pendingOperation.value = operation;
  forcePrompt.value = true;
}
function cancelForcePrompt() { if (!operationBusy.value) { forcePrompt.value = false; pendingOperation.value = null; } }
async function confirmForceClose() {
  if (!pendingOperation.value || operationBusy.value) return;
  operationBusy.value = true;
  try { forcePrompt.value = false; await pendingOperation.value(); }
  catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { operationBusy.value = false; pendingOperation.value = null; }
}
async function restoreSelected() {
  if (!recoverySelected.value.length) { notify("请至少选择一条主会话", "warning"); return; }
  const ids = [...recoverySelected.value];
  requestCloseAndRun("恢复会话", async () => {
    const targetProvider = recoveryProviderId.value;
    if (!targetProvider) throw new Error("未选择恢复目标供应商");
    if (workspace.currentProviderId?.toLowerCase() !== targetProvider.toLowerCase()) {
      await workspace.switchProvider(targetProvider);
    }
    workspace.selectedThreadIds = ids;
    const preview = await workspace.previewReplication();
    if (!preview.createCount) { notify("没有可恢复的会话", "warning"); return; }
    const result = await workspace.executeReplication(`recovery-${Date.now()}`, true);
    recoverySelected.value = [];
    notify(`恢复完成：已创建 ${result.created.length} 条会话`, "success");
  });
}
async function deleteSelectedCleanup() {
  const providerId = cleanupProviderId.value;
  const selectedIds = [...cleanupSelected.value];
  if (!providerId || !selectedIds.length) {
    notify("请至少选择一条要删除的会话", "warning");
    return;
  }
  const selectedSessions = cleanupSessions.value.filter((session) => selectedIds.includes(session.threadId));
  const childIds = selectedSessions.filter((session) => statusLabel(session) === "子会话").map((session) => session.threadId);
  const archivedIds = selectedSessions.filter((session) => statusLabel(session) === "已归档").map((session) => session.threadId);
  const availableIds = new Set<string>();
  if (workspace.activeProfileId && childIds.length) {
    const preview = await backend.invalidChildCleanupPreview(workspace.activeProfileId, providerId);
    preview.items.forEach((item) => availableIds.add(item.threadId));
  }
  if (workspace.activeProfileId && archivedIds.length) {
    const preview = await backend.archiveCleanupPreview(workspace.activeProfileId, providerId);
    preview.items.forEach((item) => availableIds.add(item.threadId));
  }
  const validSelectedIds = selectedIds.filter((id) => availableIds.has(id));
  if (validSelectedIds.length !== selectedIds.length) {
    cleanupSelected.value = validSelectedIds;
    notify("部分会话已发生变化，请刷新后重试", "warning");
    return;
  }
  requestCloseAndRun("删除选中会话", async () => {
    let deletedCount = 0;
    if (childIds.length) {
      const result = await workspace.cleanupInvalidChildSessions(providerId, childIds, true);
      deletedCount += result.deleted.length;
    }
    if (archivedIds.length) {
      const result = await workspace.cleanupArchivedSessions(providerId, archivedIds, true);
      deletedCount += result.deleted.length;
    }
    notify(`会话清理完成：已删除 ${deletedCount} 条`, "success");
    cleanupSelected.value = [];
    const provider = workspace.providerBuckets.find((item) => item.providerId === providerId);
    if (provider) await openCleanup(provider);
  });
}
async function cleanChildren() {
  const providerId = cleanupProviderId.value;
  if (!providerId || !workspace.activeProfileId) return;
  try {
    const preview = await backend.invalidChildCleanupPreview(workspace.activeProfileId, providerId);
    if (!preview.totalCount) { notify("当前供应商没有可清理的子会话"); return; }
    requestCloseAndRun("清理子会话", async () => {
      const result = await workspace.cleanupInvalidChildSessions(providerId, preview.items.map((item) => item.threadId), true);
      notify(`子会话清理完成：已删除 ${result.deleted.length} 条`, "success");
      const provider = workspace.providerBuckets.find((item) => item.providerId === providerId);
      if (provider) await openCleanup(provider);
    });
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
}
async function cleanArchived() {
  const providerId = cleanupProviderId.value;
  if (!providerId || !workspace.activeProfileId) return;
  try {
    const preview = await backend.archiveCleanupPreview(workspace.activeProfileId, providerId);
    if (!preview.totalCount) { notify("当前供应商没有可清理的归档会话"); return; }
    requestCloseAndRun("清理归档", async () => {
      const result = await workspace.cleanupArchivedSessions(providerId, preview.items.map((item) => item.threadId), true);
      notify(`归档清理完成：已删除 ${result.deleted.length} 条`, "success");
      const provider = workspace.providerBuckets.find((item) => item.providerId === providerId);
      if (provider) await openCleanup(provider);
    });
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
}

const updateTooltip = computed(() => updateInfo.value ? `当前版本 v${appVersion}，发现新版本 v${updateInfo.value.version}` : `当前版本 v${appVersion}，点击检查更新`);
async function checkUpdate() {
  if (updateChecking.value || updateInstalling.value) return;
  updateChecking.value = true;
  try { updateInfo.value = await appUpdater.check(); updateOpen.value = Boolean(updateInfo.value); if (!updateInfo.value) notify(`当前已是最新版本 ${appVersion}`, "success"); }
  catch (error) { notify(`检查更新失败：${error instanceof Error ? error.message : String(error)}`, "danger"); }
  finally { updateChecking.value = false; }
}
async function installUpdate() {
  if (!updateInfo.value || updateInstalling.value) return;
  updateInstalling.value = true;
  try { await appUpdater.install(() => {}); }
  catch (error) { notify(`升级失败：${error instanceof Error ? error.message : String(error)}`, "danger"); updateInstalling.value = false; }
}
onMounted(async () => {
  window.addEventListener("keydown", handleKeydown);
  document.addEventListener("pointerdown", closeStorageMenuFromOutside);
  if (isTauri()) unlistenTrayUpdate = await listen("app://check-update", () => { void checkUpdate(); });
  try {
    await workspace.initialize();
    // workspace.initialize loads the provider list; this second pass also
    // resolves each provider's configuration and total session size before the
    // initialization mask is removed.
    await refreshProviders(false);
  } finally {
    initializingApp.value = false;
  }
});
onUnmounted(() => { window.removeEventListener("keydown", handleKeydown); document.removeEventListener("pointerdown", closeStorageMenuFromOutside); unlistenTrayUpdate?.(); messages.dispose(); });
</script>

<template>
  <div class="app-frame" @contextmenu.prevent>
    <DesktopTitlebar />
    <main class="app-content" :aria-busy="initializingApp ? 'true' : 'false'">
      <section ref="storageRoot" class="storage-bar">
        <button type="button" class="storage-select" data-testid="storage-location-switch" :disabled="!hasMultipleStorageLocations" :aria-expanded="storageMenuOpen && hasMultipleStorageLocations" aria-haspopup="menu" @click="toggleStorageMenu">
          <Database :size="15" aria-hidden="true" />
          <span class="storage-label">配置目录</span>
          <span class="storage-select-path">{{ workspace.activeProfile?.codexHome ?? "未发现存储位置" }}</span>
          <ChevronsUpDown :size="14" aria-hidden="true" />
        </button>
        <button type="button" class="icon-button storage-refresh" data-testid="rediscover-providers" aria-label="刷新当前配置目录" :disabled="providersRefreshing" @click="refreshProviders()"><RefreshCw :size="16" :class="{ spinning: providersRefreshing }" /></button>
        <div v-if="storageMenuOpen && hasMultipleStorageLocations" class="storage-location-menu" role="menu">
          <button v-for="profile in workspace.profiles" :key="profile.id" type="button" class="storage-location-option" :class="{ active: profile.id === workspace.activeProfileId }" role="menuitemradio" :aria-checked="profile.id === workspace.activeProfileId" @click="selectStorageLocation(profile.id)">
            <Database :size="15" aria-hidden="true" />
            <span class="storage-location-copy"><strong>{{ profile.name }}</strong><small>{{ profile.codexHome }}</small></span>
            <Check v-if="profile.id === workspace.activeProfileId" class="storage-location-check" :size="15" aria-hidden="true" />
          </button>
        </div>
      </section>
      <div v-if="workspace.error" class="inline-error"><CircleAlert :size="16" />{{ workspace.error }}</div>
      <section class="provider-cards" aria-label="供应商列表">
        <article v-for="provider in workspace.providerBuckets" :key="provider.providerId" class="provider-card" :class="{ 'provider-card--current': provider.isCurrent }">
          <header class="provider-card-header"><div class="provider-card-name"><OpenAILogo v-if="isOfficial(provider.providerId)" :size="20" /><Server v-else :size="19" /><strong>{{ providerDisplayName(provider.providerId) }}</strong></div><button type="button" class="provider-use-button" :class="`provider-use-button--${providerUseState(provider)}`" :disabled="provider.isCurrent || provider.configured === false || workspace.providerConfigSwitching" @click="useProvider(provider)">{{ provider.isCurrent ? "正在使用" : provider.configured === false ? "暂无配置" : "立即使用" }}</button></header>
          <div class="provider-card-info">
            <div class="provider-card-meta">
              <div v-if="isOfficial(provider.providerId)" class="provider-stat"><span>认证状态</span><strong>{{ authStatusLabel(providerConfigs[provider.providerId]?.authStatus, provider.configured) }}</strong></div>
              <template v-else><div class="provider-stat provider-stat--copyable"><span>API 地址</span><AppTooltip class="provider-copy-tooltip" :content="providerBaseUrlValue(provider.providerId)"><button type="button" class="provider-copy-value" :disabled="providerBaseUrlValue(provider.providerId) === '未配置'" aria-label="复制 API 地址" @click="copyTextValue('API 地址', providerBaseUrlValue(provider.providerId))"><strong>{{ providerBaseUrlValue(provider.providerId) }}</strong></button></AppTooltip></div><div class="provider-stat provider-stat--copyable"><span>API 密钥</span><AppTooltip class="provider-copy-tooltip" :content="providerApiKeyValue(provider.providerId)"><button type="button" class="provider-copy-value" :disabled="providerApiKeyValue(provider.providerId) === '未配置'" aria-label="复制 API 密钥" @click="copyTextValue('API 密钥', providerApiKeyValue(provider.providerId))"><strong>{{ providerApiKeyValue(provider.providerId) }}</strong></button></AppTooltip></div></template>
            </div>
            <div class="provider-card-stats" aria-label="会话统计">
              <div class="provider-stat"><span>主会话</span><strong>{{ provider.activeRootThreadCount }}</strong></div><div class="provider-stat"><span>归档会话</span><strong>{{ provider.archivedThreadCount }}</strong></div><div class="provider-stat"><span>子会话</span><strong>{{ provider.internalThreadCount }}</strong></div><div class="provider-stat"><span>总会话大小</span><strong>{{ formatBytes(providerTotalSize(provider.providerId)) }}</strong></div>
            </div>
          </div>
          <footer class="provider-card-actions" :class="{ 'provider-card-actions--four': provider.isCurrent && !isOfficial(provider.providerId), 'provider-card-actions--two': !provider.isCurrent && !isOfficial(provider.providerId), 'provider-card-actions--one': isOfficial(provider.providerId) }"><button v-if="!isOfficial(provider.providerId)" type="button" class="card-action-button" :aria-label="provider.configured === false ? '添加供应商配置' : '修改供应商配置'" @click="openConfig('edit', provider)"><Plus v-if="provider.configured === false" :size="14" /><Edit3 v-else :size="14" />{{ provider.configured === false ? '添加配置' : '修改配置' }}</button><button v-if="provider.isCurrent" type="button" class="card-action-button" aria-label="恢复当前配置目录下的会话" @click="openRecovery(provider)"><History :size="14" />会话恢复</button><button type="button" class="card-action-button" aria-label="清理当前配置目录下的会话" @click="openCleanup(provider)"><Trash2 :size="14" />会话清理</button><button type="button" class="card-action-button" aria-label="刷新当前供应商信息" :disabled="providerRefreshing[provider.providerId]" @click="refreshProvider(provider)"><RefreshCw :size="14" :class="{ spinning: providerRefreshing[provider.providerId] }" />刷新</button></footer>
        </article>
        <button type="button" class="add-provider-button" data-testid="add-provider" @click="openConfig('add')"><Plus :size="17" />添加配置</button>
      </section>
      <div v-if="initializingApp" class="initialization-overlay" role="status" aria-live="polite" aria-label="初始化"><div class="initialization-loader" aria-hidden="true"><span class="initialization-loader-halo"></span><span class="initialization-loader-ring"></span><LoaderCircle class="initialization-loader-icon" :size="27" :stroke-width="1.8" /></div><span class="initialization-label">初始化</span><span class="initialization-dots" aria-hidden="true"><i></i><i></i><i></i></span></div>
    </main>
    <footer class="app-footer"><button type="button" aria-label="打开 711EV 导航" @click="openExternal('https://www.711ev.com/')"><Info :size="14" />关于我们</button><button type="button" aria-label="打开推荐梯子" @click="openExternal('https://www.tntv2.net/auth/register?code=oow59s')"><Rocket :size="14" />推荐梯子</button><button type="button" aria-label="打开 711EV 中转站" @click="openExternal('https://ai.711ev.com/')"><Server :size="14" />中转站</button><AppTooltip :content="updateTooltip" placement="top-end"><button type="button" :disabled="updateChecking || updateInstalling" data-testid="check-application-update" @click="checkUpdate"><LoaderCircle v-if="updateChecking" :size="14" class="spinning" /><RefreshCw v-else :size="14" />检查更新</button></AppTooltip></footer>

    <ModalDialog v-if="configDialog" :title="configDialog === 'add' ? '添加配置' : '修改配置'" modal-class="provider-config-modal" :close-disabled="configSaving" @close="closeConfig"><button v-if="configDialog === 'add'" type="button" class="provider-template-option" :class="{ active: configUse711Ev }" role="switch" :aria-checked="configUse711Ev" @click="toggle711EvPreset"><strong>使用711EV配置</strong><span class="provider-toggle-switch" :class="{ active: configUse711Ev }" aria-hidden="true"><span /></span></button><div class="provider-config-fields"><label>供应商名称<input v-model="configDraft.providerId" :disabled="configDialog === 'edit'" placeholder="例如 711EV-Codex" /></label><label>API 地址<input v-model="configDraft.baseUrl" placeholder="https://api.example.com/v1" /></label><label>API 密钥<div class="secret-field"><input v-model="configDraft.apiKey" :type="configKeyVisible ? 'text' : 'password'" autocomplete="new-password" placeholder="sk-xxxxxxxx" /><button type="button" class="secret-toggle" :aria-label="configKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'" :title="configKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'" @click="configKeyVisible = !configKeyVisible"><EyeOff v-if="configKeyVisible" :size="16" /><Eye v-else :size="16" /></button></div></label></div><template #actions><div class="modal-actions"><button class="secondary-button" :disabled="configSaving" @click="closeConfig">取消</button><span class="spacer" /><button class="primary-button" :disabled="configSaving" @click="saveConfig"><LoaderCircle v-if="configSaving" :size="15" class="spinning" /><span v-else>保存配置</span></button></div></template></ModalDialog>
     <ModalDialog v-if="recoveryOpen" title="选择要恢复的会话" modal-class="recovery-modal" :close-disabled="operationBusy" @close="closeRecovery"><div v-if="recoveryLoading" class="empty-state"><LoaderCircle :size="22" class="spinning" />正在读取会话</div><div v-else class="recovery-list"><section v-for="group in recoveryGroups" :key="group.provider" class="recovery-group"><h3><span class="recovery-group-heading"><span>{{ providerDisplayName(group.provider) }}</span><small>{{ group.sessions.length }} 条</small></span><button type="button" class="recovery-group-toggle" :class="{ 'is-collapsed': isRecoveryGroupCollapsed(group.provider) }" :aria-label="isRecoveryGroupCollapsed(group.provider) ? `展开 ${providerDisplayName(group.provider)} 会话` : `折叠 ${providerDisplayName(group.provider)} 会话`" :aria-expanded="!isRecoveryGroupCollapsed(group.provider)" @click="toggleRecoveryGroup(group.provider)"><ChevronDown :size="15" aria-hidden="true" /></button></h3><template v-if="!isRecoveryGroupCollapsed(group.provider)"><label v-for="session in group.sessions" :key="session.threadId" class="recovery-row"><input type="checkbox" :checked="recoverySelected.includes(session.threadId)" :disabled="statusLabel(session) !== '主会话' || operationBusy" @change="toggleRecoverySession(session, group.provider)" /><span class="recovery-title"><AppTooltip class="recovery-value-tooltip" :content="session.title"><strong>{{ session.title }}</strong></AppTooltip><AppTooltip class="recovery-value-tooltip" :content="session.cwd ?? session.threadId"><small><Folder :size="12" />{{ session.cwd ?? session.threadId }}</small></AppTooltip></span><span class="action-badge" :title="statusLabel(session)">{{ statusLabel(session) }}</span><span class="recovery-size" :title="formatBytes(session.sizeBytes)">{{ formatBytes(session.sizeBytes) }}</span></label></template></section></div><template #actions><div class="modal-actions recovery-actions"><button class="secondary-button" :disabled="operationBusy" @click="closeRecovery">取消</button><span class="spacer" /><button class="primary-button" :disabled="!recoverySelected.length || operationBusy" @click="restoreSelected"><Copy :size="15" />恢复选中</button></div></template></ModalDialog>
    <ModalDialog v-if="cleanupOpen" title="会话清理" modal-class="recovery-modal" :close-disabled="operationBusy" @close="closeCleanup"><div v-if="cleanupLoading" class="empty-state"><LoaderCircle :size="22" class="spinning" />正在读取会话</div><div v-else class="recovery-list"><section class="recovery-group"><h3><span class="recovery-group-heading"><span>{{ providerDisplayName(cleanupProviderId ?? '') }}</span><small>{{ cleanupSessions.length }} 条</small></span></h3><label v-for="session in cleanupSessions" :key="session.threadId" class="recovery-row"><input type="checkbox" :checked="cleanupSelected.includes(session.threadId)" :disabled="operationBusy" @change="toggleCleanupSession(session)" /><span class="recovery-title"><AppTooltip class="recovery-value-tooltip" :content="session.title"><strong>{{ session.title }}</strong></AppTooltip><AppTooltip class="recovery-value-tooltip" :content="session.cwd ?? session.threadId"><small><Folder :size="12" />{{ session.cwd ?? session.threadId }}</small></AppTooltip></span><span class="action-badge" :title="statusLabel(session)">{{ statusLabel(session) }}</span><span class="recovery-size" :title="formatBytes(session.sizeBytes)">{{ formatBytes(session.sizeBytes) }}</span></label></section></div><template #actions><div class="modal-actions recovery-actions"><button class="secondary-button" :disabled="operationBusy" @click="closeCleanup">取消</button><span class="spacer" /><button class="primary-button" :disabled="!cleanupSelected.length || operationBusy" @click="deleteSelectedCleanup"><Trash2 :size="15" />删除选中</button></div></template></ModalDialog>
    <ModalDialog v-if="forcePrompt" :title="`关闭 ChatGPT 后继续${pendingOperationLabel}`" modal-class="force-close-modal" backdrop-class="high-priority" :close-disabled="operationBusy" @close="cancelForcePrompt"><div class="force-close-warning"><TriangleAlert :size="18" /><span>执行{{ pendingOperationLabel }}前需要关闭 ChatGPT 客户端，请先保存未完成的工作。点击“立即关闭”后，工具会关闭客户端并继续当前操作。</span></div><template #actions><div class="modal-actions"><button class="secondary-button" :disabled="operationBusy" @click="cancelForcePrompt">取消</button><span class="spacer" /><button class="primary-button" :disabled="operationBusy" @click="confirmForceClose"><LoaderCircle v-if="operationBusy" :size="15" class="spinning" /><span>{{ operationBusy ? '处理中…' : '立即关闭' }}</span></button></div></template></ModalDialog>
    <ModalDialog v-if="switchPrompt" title="是否重启 ChatGPT？" :close-disabled="switchRestarting" @close="switchPrompt = null"><p>配置已写入当前 CODEX_HOME。重启 ChatGPT 后，新供应商配置才会生效。</p><template #actions><div class="modal-actions"><button class="secondary-button" :disabled="switchRestarting" @click="switchPrompt = null">暂不重启</button><span class="spacer" /><button class="primary-button" :disabled="switchRestarting" @click="restartAfterSwitch"><LoaderCircle v-if="switchRestarting" :size="15" class="spinning" /><RefreshCw v-else :size="15" />立即重启</button></div></template></ModalDialog>
    <ModalDialog v-if="updateOpen && updateInfo" :title="`发现新版本 ${updateInfo.version}`" :close-disabled="updateInstalling" @close="updateOpen = false"><p>{{ updateInfo.body || '发现新版本，建议立即更新。' }}</p><template #actions><div class="modal-actions"><button class="secondary-button" @click="updateOpen = false">稍后</button><span class="spacer" /><button class="primary-button" :disabled="updateInstalling" @click="installUpdate">{{ updateInstalling ? '正在更新…' : '立即更新' }}</button></div></template></ModalDialog>
    <TransitionGroup name="message" tag="div" class="app-message-region" role="status" aria-live="polite" aria-relevant="additions text"><div v-for="message in messages.messages" :key="message.id" class="app-message-notice"><div class="app-message" :class="`app-message--${message.tone}`" @mouseenter="messages.pauseMessage(message.id)" @mouseleave="messages.resumeMessage(message.id)"><CircleCheck v-if="message.tone === 'success'" class="app-message-icon app-message-icon--success" :size="15" aria-hidden="true" /><CircleAlert v-else-if="message.tone === 'danger'" class="app-message-icon app-message-icon--danger" :size="15" aria-hidden="true" /><TriangleAlert v-else-if="message.tone === 'warning'" class="app-message-icon app-message-icon--warning" :size="15" aria-hidden="true" /><Info v-else class="app-message-icon app-message-icon--info" :size="15" aria-hidden="true" /><span>{{ message.content }}</span></div></div></TransitionGroup>
  </div>
</template>
