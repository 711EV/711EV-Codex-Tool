<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Archive, Check, ChevronsUpDown, CircleAlert, CircleCheck, Copy, Database, Download, Edit3, Eye, EyeOff, Folder, Info, LoaderCircle, Plus, RefreshCw, Rocket, Server, Trash2, TriangleAlert, X } from "lucide-vue-next";
import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import DesktopTitlebar from "./components/DesktopTitlebar.vue";
import AppTooltip from "./components/AppTooltip.vue";
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
const recoverySelected = ref<string[]>([]);
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
const providerSizes = ref<Record<string, number>>({});
const providerRefreshing = ref<Record<string, boolean>>({});
const storageRoot = ref<HTMLElement | null>(null);
const storageMenuOpen = ref(false);
const hasMultipleStorageLocations = computed(() => workspace.profiles.length > 1);
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
function providerType(providerId: string) { return isOfficial(providerId) ? "官方" : "中转"; }
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
function maskApiKey(value: string | null | undefined) {
  const key = value?.trim();
  if (!key) return null;
  if (/[•*]/.test(key)) return key;
  if (key.length <= 8) return "••••";
  return `${key.slice(0, 3)}••••${key.slice(-4)}`;
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

async function openExternal(url: string) {
  try {
    if (isTauri()) await openUrl(url); else window.open(url, "_blank", "noopener,noreferrer");
  } catch (error) { notify(`无法打开链接：${error instanceof Error ? error.message : String(error)}`, "danger"); }
}
async function refreshProviders(showMessage = true) {
  await workspace.refreshProviders();
  const profileId = workspace.activeProfileId;
  if (profileId) {
    const entries = await Promise.all(workspace.providerBuckets.map(async (provider) => {
      try {
        const [config, snapshot] = await Promise.all([
          backend.providerConfigRead(profileId, provider.providerId),
          backend.providerWorkspace(profileId, provider.providerId),
        ]);
        return [provider.providerId, { config: { baseUrl: config.baseUrl, apiKeyMasked: maskApiKey(config.apiKeyMasked), authStatus: config.officialAuthSnapshotStatus, configured: config.configured }, size: snapshot.providerSessions.reduce((sum, session) => sum + session.sizeBytes, 0) }] as const;
      } catch { return [provider.providerId, { config: { baseUrl: null, apiKeyMasked: null, authStatus: null, configured: false }, size: 0 }] as const; }
    }));
    providerConfigs.value = Object.fromEntries(entries.map(([id, value]) => [id, value.config]));
    providerSizes.value = Object.fromEntries(entries.map(([id, value]) => [id, value.size]));
  } else {
    providerConfigs.value = {};
    providerSizes.value = {};
  }
  if (showMessage && !workspace.error) notify("供应商列表已刷新", "success");
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
    const freshBucket = snapshot.providerBuckets.find((item) => item.providerId === providerId);
    const bucketIndex = workspace.providerBuckets.findIndex((item) => item.providerId === providerId);
    if (freshBucket && bucketIndex >= 0) {
      workspace.providerBuckets[bucketIndex] = { ...workspace.providerBuckets[bucketIndex], ...freshBucket };
    }
    providerConfigs.value = {
      ...providerConfigs.value,
      [providerId]: {
        baseUrl: config.baseUrl,
        apiKeyMasked: maskApiKey(config.apiKeyMasked),
        authStatus: config.officialAuthSnapshotStatus,
        configured: config.configured,
      },
    };
    providerSizes.value = {
      ...providerSizes.value,
      [providerId]: snapshot.providerSessions.reduce((sum, session) => sum + session.sizeBytes, 0),
    };
    notify(`${providerId} 信息已刷新`, "success");
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
  recoveryLoading.value = true;
  recoveryOpen.value = true;
  try {
    const ids = [...new Set(workspace.providerBuckets.map((item) => item.providerId))];
    recoveryGroups.value = await Promise.all(ids.map(async (id) => ({ provider: id, sessions: (await backend.providerWorkspace(provider.profileId, id)).providerSessions })));
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
  finally { recoveryLoading.value = false; }
}
function closeRecovery() { if (!operationBusy.value) recoveryOpen.value = false; }
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
async function cleanChildren() {
  const providerId = recoveryProviderId.value;
  if (!providerId || !workspace.activeProfileId) return;
  try {
    const preview = await backend.invalidChildCleanupPreview(workspace.activeProfileId, providerId);
    if (!preview.totalCount) { notify("当前供应商没有可清理的子会话"); return; }
    requestCloseAndRun("清理子会话", async () => {
      const result = await workspace.cleanupInvalidChildSessions(providerId, preview.items.map((item) => item.threadId), true);
      notify(`子会话清理完成：已删除 ${result.deleted.length} 条`, "success");
      const provider = workspace.providerBuckets.find((item) => item.providerId === providerId);
      if (provider) await openRecovery(provider);
    });
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
}
async function cleanArchived() {
  const providerId = recoveryProviderId.value;
  if (!providerId || !workspace.activeProfileId) return;
  try {
    const preview = await backend.archiveCleanupPreview(workspace.activeProfileId, providerId);
    if (!preview.totalCount) { notify("当前供应商没有可清理的归档会话"); return; }
    requestCloseAndRun("清理归档", async () => {
      const result = await workspace.cleanupArchivedSessions(providerId, preview.items.map((item) => item.threadId), true);
      notify(`归档清理完成：已删除 ${result.deleted.length} 条`, "success");
      const provider = workspace.providerBuckets.find((item) => item.providerId === providerId);
      if (provider) await openRecovery(provider);
    });
  } catch (error) { notify(error instanceof Error ? error.message : String(error), "danger"); }
}

const updateTooltip = computed(() => updateInfo.value ? `当前版本 ${appVersion}，发现新版本 ${updateInfo.value.version}` : `当前版本 ${appVersion}，点击检查更新`);
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
  await workspace.initialize();
  await refreshProviders(false);
});
onUnmounted(() => { window.removeEventListener("keydown", handleKeydown); document.removeEventListener("pointerdown", closeStorageMenuFromOutside); unlistenTrayUpdate?.(); messages.dispose(); });
</script>

<template>
  <div class="app-frame" @contextmenu.prevent>
    <DesktopTitlebar />
    <main class="app-content">
      <section ref="storageRoot" class="storage-bar">
        <button type="button" class="storage-select" data-testid="storage-location-switch" :disabled="!hasMultipleStorageLocations" :aria-expanded="storageMenuOpen && hasMultipleStorageLocations" aria-haspopup="menu" @click="toggleStorageMenu">
          <Database :size="15" aria-hidden="true" />
          <span class="storage-label">配置目录</span>
          <span class="storage-select-path">{{ workspace.activeProfile?.codexHome ?? "未发现存储位置" }}</span>
          <ChevronsUpDown :size="14" aria-hidden="true" />
        </button>
        <button type="button" class="icon-button storage-refresh" data-testid="rediscover-providers" aria-label="刷新当前配置目录" :disabled="workspace.loading" @click="refreshProviders()"><RefreshCw :size="16" :class="{ spinning: workspace.loading }" /></button>
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
          <header class="provider-card-header"><div class="provider-card-name"><OpenAILogo v-if="isOfficial(provider.providerId)" :size="20" /><Server v-else :size="19" /><strong>{{ provider.providerId }}</strong></div><button type="button" class="provider-use-button" :class="`provider-use-button--${providerUseState(provider)}`" :disabled="provider.isCurrent || provider.configured === false || workspace.providerConfigSwitching" @click="useProvider(provider)">{{ provider.isCurrent ? "正在使用" : provider.configured === false ? "暂无配置" : "立即使用" }}</button></header>
          <div class="provider-card-info">
            <div class="provider-card-meta">
              <div class="provider-stat"><span>供应商类型</span><strong>{{ providerType(provider.providerId) }}</strong></div>
              <div v-if="isOfficial(provider.providerId)" class="provider-stat"><span>认证状态</span><strong>{{ authStatusLabel(providerConfigs[provider.providerId]?.authStatus, provider.configured) }}</strong></div>
              <template v-else><div class="provider-stat"><span>API 地址</span><strong>{{ providerConfigs[provider.providerId]?.baseUrl ?? '未配置' }}</strong></div><div class="provider-stat"><span>API 密钥</span><strong>{{ providerConfigs[provider.providerId]?.apiKeyMasked ?? '未配置' }}</strong></div></template>
            </div>
            <div class="provider-card-stats" aria-label="会话统计">
              <div class="provider-stat"><span>主会话</span><strong>{{ provider.activeRootThreadCount }}</strong></div><div class="provider-stat"><span>归档会话</span><strong>{{ provider.archivedThreadCount }}</strong></div><div class="provider-stat"><span>子会话</span><strong>{{ provider.internalThreadCount }}</strong></div><div class="provider-stat"><span>总会话大小</span><strong>{{ formatBytes(providerTotalSize(provider.providerId)) }}</strong></div>
            </div>
          </div>
          <footer class="provider-card-actions"><button type="button" class="card-action-button" :disabled="isOfficial(provider.providerId)" :aria-label="isOfficial(provider.providerId) ? '官方供应商不支持修改' : '修改供应商配置'" @click="openConfig('edit', provider)"><Edit3 :size="14" />修改配置</button><button type="button" class="card-action-button" aria-label="恢复当前配置目录下的会话" @click="openRecovery(provider)"><Copy :size="14" />会话恢复</button><button type="button" class="card-action-button" aria-label="刷新当前供应商信息" :disabled="providerRefreshing[provider.providerId]" @click="refreshProvider(provider)"><RefreshCw :size="14" :class="{ spinning: providerRefreshing[provider.providerId] }" />刷新</button></footer>
        </article>
        <button type="button" class="add-provider-button" data-testid="add-provider" @click="openConfig('add')"><Plus :size="17" />添加配置</button>
      </section>
    </main>
    <footer class="app-footer"><button type="button" aria-label="打开 711EV 导航" @click="openExternal('https://www.711ev.com/')"><Info :size="14" />关于我们</button><button type="button" aria-label="打开推荐梯子" @click="openExternal('https://www.tntv2.net/auth/register?code=oow59s')"><Rocket :size="14" />推荐梯子</button><button type="button" aria-label="打开 711EV 中转站" @click="openExternal('https://ai.711ev.com/')"><Server :size="14" />中转站</button><AppTooltip :content="updateTooltip" placement="top-end"><button type="button" :disabled="updateChecking || updateInstalling" data-testid="check-application-update" @click="checkUpdate"><LoaderCircle v-if="updateChecking" :size="14" class="spinning" /><RefreshCw v-else :size="14" />检查更新</button></AppTooltip></footer>

    <div v-if="configDialog" class="modal-backdrop"><section class="modal provider-config-modal"><div class="modal-heading"><div><p class="eyebrow">供应商配置</p><h2>{{ configDialog === 'add' ? '添加配置' : '修改配置' }}</h2></div><button class="icon-button" :disabled="configSaving" aria-label="关闭" @click="closeConfig"><X :size="18" /></button></div><div v-if="configDialog === 'add'" class="provider-template-option"><strong>使用711EV配置</strong><button type="button" class="provider-toggle-switch" :class="{ active: configUse711Ev }" role="switch" :aria-checked="configUse711Ev" :title="configUse711Ev ? '关闭711EV预设' : '使用711EV预设'" @click="toggle711EvPreset"><span /></button></div><div class="provider-config-fields"><label>供应商名称<input v-model="configDraft.providerId" :disabled="configDialog === 'edit'" placeholder="例如 711EV-Codex" /></label><label>API 地址<input v-model="configDraft.baseUrl" placeholder="https://api.example.com/v1" /></label><label>API 密钥<div class="secret-field"><input v-model="configDraft.apiKey" :type="configKeyVisible ? 'text' : 'password'" autocomplete="new-password" placeholder="sk-xxxxxxxx" /><button type="button" class="secret-toggle" :aria-label="configKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'" :title="configKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'" @click="configKeyVisible = !configKeyVisible"><EyeOff v-if="configKeyVisible" :size="16" /><Eye v-else :size="16" /></button></div></label></div><div class="modal-actions"><button class="secondary-button" :disabled="configSaving" @click="closeConfig">取消</button><button class="primary-button" :disabled="configSaving" @click="saveConfig"><LoaderCircle v-if="configSaving" :size="15" class="spinning" /><span v-else>保存配置</span></button></div></section></div>
     <div v-if="recoveryOpen" class="modal-backdrop"><section class="modal recovery-modal"><div class="modal-heading"><div><p class="eyebrow">{{ recoveryProviderId }} · 会话恢复</p><h2>选择要恢复的会话</h2></div><button class="icon-button" :disabled="operationBusy" aria-label="关闭" @click="closeRecovery"><X :size="18" /></button></div><div v-if="recoveryLoading" class="empty-state"><LoaderCircle :size="22" class="spinning" />正在读取会话</div><div v-else class="recovery-list"><section v-for="group in recoveryGroups" :key="group.provider" class="recovery-group"><h3>{{ group.provider }} <small>{{ group.sessions.length }} 条</small></h3><label v-for="session in group.sessions" :key="session.threadId" class="recovery-row" :class="{ disabled: group.provider.toLowerCase() === recoveryProviderId?.toLowerCase() || statusLabel(session) !== '主会话' }"><input type="checkbox" :checked="recoverySelected.includes(session.threadId)" :disabled="group.provider.toLowerCase() === recoveryProviderId?.toLowerCase() || statusLabel(session) !== '主会话' || operationBusy" @change="toggleRecoverySession(session, group.provider)" /><span class="recovery-title"><AppTooltip class="recovery-value-tooltip" :content="session.title"><strong>{{ session.title }}</strong></AppTooltip><AppTooltip class="recovery-value-tooltip" :content="session.cwd ?? session.threadId"><small><Folder :size="12" />{{ session.cwd ?? session.threadId }}</small></AppTooltip></span><span class="action-badge" :title="statusLabel(session)">{{ statusLabel(session) }}</span><span class="recovery-size" :title="formatBytes(session.sizeBytes)">{{ formatBytes(session.sizeBytes) }}</span></label></section></div><div class="modal-actions recovery-actions"><button class="secondary-button" :disabled="operationBusy" @click="cleanChildren"><Trash2 :size="15" />清理子会话</button><button class="secondary-button" :disabled="operationBusy" @click="cleanArchived"><Archive :size="15" />清理归档</button><span class="spacer" /><button class="primary-button" :disabled="!recoverySelected.length || operationBusy" @click="restoreSelected"><Copy :size="15" />恢复选中</button></div></section></div>
    <div v-if="forcePrompt" class="modal-backdrop high-priority"><section class="modal force-close-modal"><div class="modal-heading"><div><p class="eyebrow">需要关闭 ChatGPT</p><h2>关闭 ChatGPT 后继续{{ pendingOperationLabel }}</h2></div><button class="icon-button" :disabled="operationBusy" @click="cancelForcePrompt"><X :size="18" /></button></div><div class="force-close-warning"><TriangleAlert :size="18" /><span>执行{{ pendingOperationLabel }}前需要关闭 ChatGPT 客户端，请先保存未完成的工作。点击“立即关闭”后，工具会关闭客户端并继续当前操作。</span></div><div class="modal-actions"><button class="secondary-button" :disabled="operationBusy" @click="cancelForcePrompt">取消</button><button class="primary-button" :disabled="operationBusy" @click="confirmForceClose"><LoaderCircle v-if="operationBusy" :size="15" class="spinning" /><span>{{ operationBusy ? '处理中…' : '立即关闭' }}</span></button></div></section></div>
    <div v-if="switchPrompt" class="modal-backdrop"><section class="modal"><div class="modal-heading"><div><p class="eyebrow">供应商已切换</p><h2>是否重启 ChatGPT？</h2></div><button class="icon-button" :disabled="switchRestarting" @click="switchPrompt = null"><X :size="18" /></button></div><p>配置已写入当前 CODEX_HOME。重启 ChatGPT 后，新供应商配置才会生效。</p><div class="modal-actions"><button class="secondary-button" :disabled="switchRestarting" @click="switchPrompt = null">暂不重启</button><button class="primary-button" :disabled="switchRestarting" @click="restartAfterSwitch"><LoaderCircle v-if="switchRestarting" :size="15" class="spinning" /><RefreshCw v-else :size="15" />立即重启</button></div></section></div>
    <div v-if="updateOpen && updateInfo" class="modal-backdrop"><section class="modal"><div class="modal-heading"><h2>发现新版本 {{ updateInfo.version }}</h2><button class="icon-button" :disabled="updateInstalling" @click="updateOpen = false"><X :size="18" /></button></div><p>{{ updateInfo.body || '发现新版本，建议立即更新。' }}</p><div class="modal-actions"><button class="secondary-button" @click="updateOpen = false">稍后</button><button class="primary-button" :disabled="updateInstalling" @click="installUpdate">{{ updateInstalling ? '正在更新…' : '立即更新' }}</button></div></section></div>
    <TransitionGroup name="message" tag="div" class="app-message-region" role="status" aria-live="polite" aria-relevant="additions text"><div v-for="message in messages.messages" :key="message.id" class="app-message-notice"><div class="app-message" :class="`app-message--${message.tone}`" @mouseenter="messages.pauseMessage(message.id)" @mouseleave="messages.resumeMessage(message.id)"><CircleCheck v-if="message.tone === 'success'" class="app-message-icon app-message-icon--success" :size="15" aria-hidden="true" /><CircleAlert v-else-if="message.tone === 'danger'" class="app-message-icon app-message-icon--danger" :size="15" aria-hidden="true" /><TriangleAlert v-else-if="message.tone === 'warning'" class="app-message-icon app-message-icon--warning" :size="15" aria-hidden="true" /><Info v-else class="app-message-icon app-message-icon--info" :size="15" aria-hidden="true" /><span>{{ message.content }}</span></div></div></TransitionGroup>
  </div>
</template>
