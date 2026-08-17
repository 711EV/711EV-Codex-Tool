<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  AlertTriangle,
  ArrowRightLeft,
  Archive,
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Copy,
  Download,
  Eye,
  EyeOff,
  Folder,
  GitFork,
  LoaderCircle,
  MoveRight,
  Plus,
  Pencil,
  RefreshCcw,
  RefreshCw,
  Search,
  Server,
  Trash2,
  X,
} from "lucide-vue-next";
import DesktopTitlebar from "./components/DesktopTitlebar.vue";
import OpenAILogo from "./components/OpenAILogo.vue";
import TopMenuBar from "./components/TopMenuBar.vue";
import { appUpdater } from "./services/appUpdater";
import { useWorkspaceStore } from "./stores/workspace";
import type {
  ApplicationUpdate,
  ArchiveCleanupPreview,
  InvalidChildCleanupPreview,
  ReplicationAction,
  ReplicationEligibility,
  ReplicationPreview,
  ProviderSessionRecord,
  ProviderConfigInput,
  ProviderSwitchResult,
  UpdateSyncAction,
  UpdateSyncPreview,
  UpdateDownloadProgress,
} from "./types";

const workspace = useWorkspaceStore();
const appVersion = __APP_VERSION__;
const sevenElevenProviderId = "711EV";
const sevenElevenBaseUrl = "https://ai.711ev.com/v1";
const preview = ref<ReplicationPreview | null>(null);
const previewLoading = ref(false);
const migrationPreview = ref<ReplicationPreview | null>(null);
const migrationPreviewLoading = ref(false);
const updateSyncPreview = ref<UpdateSyncPreview | null>(null);
const updateSyncPreviewLoading = ref(false);
const updateSyncResult = ref<Awaited<ReturnType<typeof workspace.syncUpdatedSessions>> | null>(null);
const archiveCleanupPreview = ref<ArchiveCleanupPreview | null>(null);
const archiveCleanupPreviewLoading = ref(false);
const archiveCleanupResult = ref<Awaited<ReturnType<typeof workspace.cleanupArchivedSessions>> | null>(null);
const childCleanupPreview = ref<InvalidChildCleanupPreview | null>(null);
const childCleanupPreviewLoading = ref(false);
const childCleanupResult = ref<Awaited<ReturnType<typeof workspace.cleanupInvalidChildSessions>> | null>(null);
const manualRefreshing = ref(false);
const applicationUpdate = ref<ApplicationUpdate | null>(null);
const updateDialogOpen = ref(false);
const updateChecking = ref(false);
const updateInstalling = ref(false);
const updateProgress = ref<UpdateDownloadProgress | null>(null);
const providerConfigDialog = ref<"add" | "edit" | null>(null);
const providerConfigKeyVisible = ref(false);
const providerConfigDraft = ref<ProviderConfigInput>({
  profileId: "",
  providerId: "",
  baseUrl: "",
  apiKey: "",
  template: "other",
});
const providerConfigIs711Ev = ref(false);
const providerSwitchDialog = ref<string | null>(null);
const providerSwitchResult = ref<ProviderSwitchResult | null>(null);
const providerSwitchRestarting = ref(false);
const forceClosePrompt = ref(false);
const restartPrompt = ref<{
  operation: "复制" | "迁移";
  completedCount: number;
  summary: string;
} | null>(null);
const pendingForceOperation = ref<
  "replication" | "migration" | "client-restart" | "provider-switch-restart" | "update-sync" | "archive-cleanup" | "child-cleanup" | null
>(null);
const notice = ref<string | null>(null);
let noticeDismissTimer: number | undefined;
const expandedSessionGroups = ref<string[]>([]);
const canPreview = computed(
  () =>
    Boolean(workspace.activeProfileId) &&
    Boolean(workspace.currentProviderId) &&
    !workspace.selectedProvider?.isCurrent &&
    workspace.selectedThreadIds.length > 0,
);
const allVisibleSelected = computed(
  () =>
    workspace.selectableSessions.length > 0 &&
    workspace.selectableSessions.every((session) =>
      workspace.selectedThreadIds.includes(session.threadId),
    ),
);
const paneActionBusy = computed(
  () =>
    workspace.loading ||
    workspace.syncing ||
    workspace.migrating ||
    workspace.cleaningArchived ||
    workspace.cleaningChildren ||
    updateSyncPreviewLoading.value ||
    archiveCleanupPreviewLoading.value ||
    childCleanupPreviewLoading.value,
);
const sessionGroups = computed(() => {
  const sessions = workspace.providerSessions;
  const sessionsById = new Map(sessions.map((session) => [session.threadId, session]));
  const childrenByRoot = new Map<string, ProviderSessionRecord[]>();
  const linkedChildIds = new Set<string>();

  for (const session of sessions) {
    if (session.sourceKind !== "internal") continue;
    const root = resolveRootSession(session, sessionsById);
    if (!root) continue;
    linkedChildIds.add(session.threadId);
    const children = childrenByRoot.get(root.threadId) ?? [];
    children.push(session);
    childrenByRoot.set(root.threadId, children);
  }

  const term = workspace.search.trim().toLocaleLowerCase();
  return sessions
    .filter((session) => !linkedChildIds.has(session.threadId))
    .map((session) => ({
      session,
      children: childrenByRoot.get(session.threadId) ?? [],
    }))
    .map((group) => {
      if (!term || sessionMatches(group.session, term)) return group;
      return {
        session: group.session,
        children: group.children.filter((session) => sessionMatches(session, term)),
      };
    })
    .filter((group) =>
      !term || sessionMatches(group.session, term) || group.children.length > 0,
    );
});

onMounted(async () => {
  window.addEventListener("keydown", handleModalKeydown);
  await workspace.initialize();
  if (import.meta.env.PROD) {
    void checkApplicationUpdate(true);
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleModalKeydown);
  if (noticeDismissTimer !== undefined) window.clearTimeout(noticeDismissTimer);
});

watch(notice, (message) => {
  if (noticeDismissTimer !== undefined) {
    window.clearTimeout(noticeDismissTimer);
    noticeDismissTimer = undefined;
  }
  if (!message) return;

  noticeDismissTimer = window.setTimeout(() => {
    notice.value = null;
    noticeDismissTimer = undefined;
  }, 5_000);
});

function handleModalKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;

  if (forceClosePrompt.value) {
    closeForcePrompt();
  } else if (applicationUpdate.value && updateDialogOpen.value) {
    void closeApplicationUpdate();
  } else if (providerSwitchResult.value) {
    dismissProviderSwitchRestart();
  } else if (providerSwitchDialog.value) {
    closeProviderSwitchDialog();
  } else if (providerConfigDialog.value) {
    closeProviderConfigDialog();
  } else if (restartPrompt.value) {
    dismissRestartPrompt();
  } else if (archiveCleanupPreview.value) {
    closeArchiveCleanupPreview();
  } else if (childCleanupPreview.value) {
    closeChildCleanupPreview();
  } else if (updateSyncPreview.value) {
    closeUpdateSyncPreview();
  } else if (migrationPreview.value) {
    closeMigrationPreview();
  } else if (preview.value) {
    preview.value = null;
  } else {
    return;
  }

  event.preventDefault();
}

function resolveRootSession(
  session: ProviderSessionRecord,
  sessionsById: Map<string, ProviderSessionRecord>,
) {
  const visited = new Set([session.threadId]);
  let parentId = session.parentThreadId;
  while (parentId && !visited.has(parentId)) {
    visited.add(parentId);
    const parent = sessionsById.get(parentId);
    if (!parent) return null;
    if (parent.sourceKind !== "internal") return parent;
    parentId = parent.parentThreadId;
  }
  return null;
}

function sessionMatches(session: ProviderSessionRecord, term: string) {
  return [session.title, session.cwd, session.threadId, session.eligibilityReason]
    .filter(Boolean)
    .some((value) => value!.toLocaleLowerCase().includes(term));
}

function sessionGroupExpanded(threadId: string) {
  return Boolean(workspace.search.trim()) || expandedSessionGroups.value.includes(threadId);
}

function toggleSessionGroup(threadId: string) {
  expandedSessionGroups.value = expandedSessionGroups.value.includes(threadId)
    ? expandedSessionGroups.value.filter((id) => id !== threadId)
    : [...expandedSessionGroups.value, threadId];
}

function formatDate(value: string | null) {
  if (!value) return "未知时间";
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

function sourceLabel(source: string) {
  return { cli: "CLI", vscode: "客户端", internal: "子agent", unknown: "未知" }[source] ?? source;
}

function sessionSourceLabel(session: ProviderSessionRecord) {
  if (session.sourceKind !== "internal") return sourceLabel(session.sourceKind);
  return session.agentNickname?.trim() || "未命名";
}

function isOfficialProvider(providerId: string) {
  return providerId.toLocaleLowerCase() === "openai";
}

function providerKindLabel(providerId: string) {
  return isOfficialProvider(providerId) ? "官方" : "中转";
}

function eligibilityLabel(value: ReplicationEligibility) {
  return {
    eligible: "可复制",
    current_provider: "当前",
    archived: "已归档",
    internal_thread: "子会话",
    invalid_rollout: "无效",
    already_replicated: "已复制",
    replica: "副本",
    replica_updated: "副本已更新",
    source_updated: "来源已更新",
  }[value];
}

function actionLabel(action: ReplicationAction) {
  return {
    create_replica: "新副本",
    skip_already_replicated: "已复制",
    source_updated: "来源已更新",
    skip_current_provider: "当前",
    skip_archived: "归档",
    skip_internal: "子会话",
    invalid: "无效",
  }[action];
}

function updateSyncActionLabel(action: UpdateSyncAction) {
  return {
    source_updated: "来源已更新",
    replica_updated: "副本已更新",
    conflict: "冲突",
    invalid: "无效",
  }[action];
}

function updateSyncRoute(item: UpdateSyncPreview["items"][number]) {
  if (item.action === "replica_updated") {
    return `${item.targetProviderId} → ${item.sourceProviderId}`;
  }
  if (item.action === "source_updated") {
    return `${item.sourceProviderId} → ${item.targetProviderId}`;
  }
  return `${item.sourceProviderId} ↔ ${item.targetProviderId}`;
}

async function openPreview() {
  previewLoading.value = true;
  try {
    preview.value = await workspace.previewReplication();
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    previewLoading.value = false;
  }
}

async function runReplication(force = false) {
  try {
    const result = await workspace.executeReplication(force);
    preview.value = null;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    const summary = result.warning
      ? `已创建 ${result.created.length} 条，失败 ${result.failed.length} 条；${result.warning}`
      : `复制完成：已创建 ${result.created.length} 条，跳过 ${result.skipped.length} 条，失败 ${result.failed.length} 条`;
    notice.value = summary;
    if (result.created.length > 0 && !result.clientRestarted) {
      restartPrompt.value = {
        operation: "复制",
        completedCount: result.created.length,
        summary,
      };
    }
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "replication";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function openMigrationPreview() {
  migrationPreviewLoading.value = true;
  try {
    migrationPreview.value = await workspace.previewReplication();
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    migrationPreviewLoading.value = false;
  }
}

function closeMigrationPreview() {
  if (workspace.migrating) return;
  migrationPreview.value = null;
}

async function runMigration(force = false) {
  try {
    const result = await workspace.executeMigration(force);
    migrationPreview.value = null;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    const summary = `迁移完成：已迁移 ${result.created.length} 条，跳过 ${result.skipped.length} 条，失败 ${result.failed.length} 条`;
    const failures = result.failed
      .slice(0, 2)
      .map((item) => `${item.title}：${item.message}`)
      .join("；");
    const resultSummary = [summary, failures, result.warning].filter(Boolean).join("；");
    notice.value = resultSummary;
    if (result.created.length > 0 && !result.clientRestarted) {
      restartPrompt.value = {
        operation: "迁移",
        completedCount: result.created.length,
        summary: resultSummary,
      };
    }
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "migration";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function runUpdateSync(force = false) {
  try {
    const result = await workspace.syncUpdatedSessions(force);
    updateSyncResult.value = result;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    if (
      result.created.length === 0 &&
      result.skipped.length === 0 &&
      result.failed.length === 0
    ) {
      notice.value = "当前没有需要同步的更新会话";
      return;
    }
    const summary = `同步完成：已更新 ${result.created.length} 条，跳过 ${result.skipped.length} 条，失败 ${result.failed.length} 条`;
    const failures = result.failed
      .slice(0, 2)
      .map((item) => `${item.title}：${item.message}`)
      .join("；");
    notice.value = [summary, failures, result.warning].filter(Boolean).join("；");
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "update-sync";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function openUpdateSyncPreview() {
  updateSyncPreviewLoading.value = true;
  updateSyncResult.value = null;
  try {
    updateSyncPreview.value = await workspace.previewUpdatedSessions();
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    updateSyncPreviewLoading.value = false;
  }
}

function closeUpdateSyncPreview() {
  if (workspace.syncing) return;
  updateSyncPreview.value = null;
  updateSyncResult.value = null;
}

async function openArchiveCleanupPreview() {
  archiveCleanupPreviewLoading.value = true;
  archiveCleanupResult.value = null;
  try {
    archiveCleanupPreview.value = await workspace.previewArchivedCleanup();
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    archiveCleanupPreviewLoading.value = false;
  }
}

function closeArchiveCleanupPreview() {
  if (workspace.cleaningArchived) return;
  archiveCleanupPreview.value = null;
  archiveCleanupResult.value = null;
}

async function runArchiveCleanup(force = false) {
  if (!archiveCleanupPreview.value) return;
  try {
    const result = await workspace.cleanupArchivedSessions(
      archiveCleanupPreview.value.providerId,
      archiveCleanupPreview.value.items.map((item) => item.threadId),
      force,
    );
    archiveCleanupResult.value = result;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    const summary = `清理完成：已删除 ${result.deleted.length} 条，失败 ${result.failed.length} 条`;
    const failures = result.failed
      .slice(0, 2)
      .map((item) => `${item.title}：${item.message}`)
      .join("；");
    notice.value = [summary, failures, result.warning].filter(Boolean).join("；");
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "archive-cleanup";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function openChildCleanupPreview() {
  childCleanupPreviewLoading.value = true;
  childCleanupResult.value = null;
  try {
    childCleanupPreview.value = await workspace.previewInvalidChildCleanup();
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    childCleanupPreviewLoading.value = false;
  }
}

function closeChildCleanupPreview() {
  if (workspace.cleaningChildren) return;
  childCleanupPreview.value = null;
  childCleanupResult.value = null;
}

async function runChildCleanup(force = false) {
  if (!childCleanupPreview.value) return;
  try {
    const result = await workspace.cleanupInvalidChildSessions(
      childCleanupPreview.value.providerId,
      childCleanupPreview.value.items.map((item) => item.threadId),
      force,
    );
    childCleanupResult.value = result;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    const summary = `子会话清理完成：已删除 ${result.deleted.length} 条，失败 ${result.failed.length} 条`;
    const failures = result.failed
      .slice(0, 2)
      .map((item) => `${item.title}：${item.message}`)
      .join("；");
    notice.value = [summary, failures, result.warning].filter(Boolean).join("；");
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "child-cleanup";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function retryForcedOperation() {
  if (pendingForceOperation.value === "provider-switch-restart") {
    await restartAfterProviderSwitch(true);
  } else if (pendingForceOperation.value === "client-restart") {
    await restartCodexDesktop(true);
  } else if (pendingForceOperation.value === "child-cleanup") {
    await runChildCleanup(true);
  } else if (pendingForceOperation.value === "archive-cleanup") {
    await runArchiveCleanup(true);
  } else if (pendingForceOperation.value === "update-sync") {
    await runUpdateSync(true);
  } else if (pendingForceOperation.value === "migration") {
    await runMigration(true);
  } else {
    await runReplication(true);
  }
}

function dismissRestartPrompt() {
  if (workspace.restartingClient) return;
  restartPrompt.value = null;
}

async function restartCodexDesktop(force = false) {
  try {
    await workspace.restartCodexClient(force);
    restartPrompt.value = null;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    notice.value = "Codex Desktop 已重启，会话列表将重新加载";
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "client-restart";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

function closeForcePrompt() {
  forceClosePrompt.value = false;
  pendingForceOperation.value = null;
}

async function openProviderConfigDialog(mode: "add" | "edit") {
  const selected = workspace.providerConfig;
  if (mode === "edit" && (!selected || isOfficialProvider(selected.providerId))) {
    notice.value = "官方供应商不需要填写 API 密钥";
    return;
  }
  const draft: ProviderConfigInput = mode === "edit" && selected
    ? {
        profileId: selected.profileId,
        providerId: selected.providerId,
        baseUrl: selected.baseUrl,
        apiKey: "",
        template: selected.providerId === "711EV" ? "711ev" : "other",
      }
    : {
        profileId: workspace.activeProfileId ?? "",
        providerId: "",
        baseUrl: "",
        apiKey: "",
        template: "other",
      };
  if (mode === "edit" && selected?.managedByTool && selected.apiKeyMasked) {
    try {
      draft.apiKey = await workspace.revealProviderApiKey(selected.profileId, selected.providerId);
    } catch (reason) {
      notice.value = reason instanceof Error ? reason.message : String(reason);
      return;
    }
  }
  providerConfigDraft.value = draft;
  providerConfigIs711Ev.value = draft.template === "711ev";
  providerConfigKeyVisible.value = false;
  providerConfigDialog.value = mode;
}

function closeProviderConfigDialog() {
  if (workspace.providerConfigSaving) return;
  providerConfigDialog.value = null;
  providerConfigKeyVisible.value = false;
  providerConfigDraft.value.apiKey = "";
}

function toggle711EvProvider() {
  providerConfigIs711Ev.value = !providerConfigIs711Ev.value;
  providerConfigDraft.value.template = providerConfigIs711Ev.value ? "711ev" : "other";
  if (!providerConfigIs711Ev.value) {
    providerConfigDraft.value.providerId = "";
    providerConfigDraft.value.baseUrl = "";
    providerConfigDraft.value.apiKey = "";
    return;
  }
  providerConfigDraft.value.providerId = sevenElevenProviderId;
  providerConfigDraft.value.baseUrl = sevenElevenBaseUrl;
}

async function saveProviderConfigDialog() {
  const draft = providerConfigDraft.value;
  if (!draft.profileId) {
    notice.value = "未选择存储位置";
    return;
  }
  draft.template = providerConfigIs711Ev.value ? "711ev" : "other";
  draft.providerId = draft.providerId.trim();
  try {
    await workspace.saveProviderConfig({ ...draft });
    providerConfigDialog.value = null;
    providerConfigKeyVisible.value = false;
    providerConfigDraft.value.apiKey = "";
    notice.value = `供应商 ${draft.providerId} 已保存，当前供应商未改变`;
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  }
}

async function requestProviderSwitch(providerId: string) {
  if (providerId === workspace.currentProviderId || workspace.providerConfigSwitching) return;
  if (workspace.selectedProviderId !== providerId) {
    await workspace.selectProvider(providerId);
  }
  if (workspace.providerConfig?.authStorage !== "file") {
    notice.value = "不支持当前认证存储方式，仅支持主 config.toml 中明确设置 cli_auth_credentials_store = \"file\"";
    return;
  }
  if (!workspace.providerConfig?.canSwitch) {
    notice.value = workspace.providerConfig?.officialAuthSnapshotStatus === "conflict"
      ? "官方凭据快照完整性异常，无法执行切换"
      : "当前供应商配置尚未满足切换条件";
    return;
  }
  providerSwitchDialog.value = providerId;
}

function requestApplyCurrentProvider() {
  const config = workspace.providerConfig;
  if (!config || !config.hasPendingChanges) return;
  if (!config.canSwitch) {
    notice.value = "不支持当前认证存储方式，仅支持主 config.toml 中明确设置 cli_auth_credentials_store = \"file\"";
    return;
  }
  providerSwitchDialog.value = config.providerId;
}

function closeProviderSwitchDialog() {
  if (!workspace.providerConfigSwitching) providerSwitchDialog.value = null;
}

async function confirmProviderSwitch() {
  if (!providerSwitchDialog.value) return;
  const providerId = providerSwitchDialog.value;
  try {
    providerSwitchResult.value = await workspace.switchProvider(providerId);
    providerSwitchDialog.value = null;
    notice.value = `已切换到 ${providerId}，配置已写入主 config.toml 和 auth.json`;
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  }
}

function dismissProviderSwitchRestart() {
  if (!providerSwitchRestarting.value) providerSwitchResult.value = null;
}

async function restartAfterProviderSwitch(force = false) {
  if (!providerSwitchResult.value) return;
  providerSwitchRestarting.value = true;
  try {
    await workspace.restartCodexClient(force);
    providerSwitchResult.value = null;
    forceClosePrompt.value = false;
    pendingForceOperation.value = null;
    notice.value = "Codex Desktop 已重启，并会读取刚刚写入的供应商配置";
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      pendingForceOperation.value = "provider-switch-restart";
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  } finally {
    providerSwitchRestarting.value = false;
  }
}

async function rediscoverProviders() {
  await workspace.refreshProviders();
  if (!workspace.error) {
    notice.value = `已重新发现当前存储位置下的 ${workspace.providerBuckets.length} 个供应商`;
  }
}

async function refreshWorkspace() {
  manualRefreshing.value = true;
  try {
    await workspace.refreshProviders();
  } finally {
    manualRefreshing.value = false;
  }
}

async function checkApplicationUpdate(silent = false) {
  if (updateChecking.value || updateInstalling.value) return;
  updateChecking.value = true;
  notice.value = null;
  try {
    applicationUpdate.value = await appUpdater.check();
    updateDialogOpen.value = Boolean(applicationUpdate.value);
    if (!applicationUpdate.value && !silent) {
      notice.value = `当前已是最新版本 ${appVersion}`;
    }
  } catch (reason) {
    if (!silent) {
      notice.value = `检查更新失败：${reason instanceof Error ? reason.message : String(reason)}`;
    }
  } finally {
    updateChecking.value = false;
  }
}

async function closeApplicationUpdate() {
  if (updateInstalling.value) return;
  await appUpdater.clear();
  updateDialogOpen.value = false;
  updateProgress.value = null;
}

async function installApplicationUpdate() {
  if (!applicationUpdate.value || updateInstalling.value) return;
  updateInstalling.value = true;
  updateProgress.value = { downloadedBytes: 0, totalBytes: null, percent: null };
  try {
    await appUpdater.install((progress) => {
      updateProgress.value = progress;
    });
  } catch (reason) {
    notice.value = `升级失败：${reason instanceof Error ? reason.message : String(reason)}`;
    updateInstalling.value = false;
  }
}
</script>

<template>
  <div class="app-frame" @contextmenu.prevent>
    <DesktopTitlebar />
    <TopMenuBar
      :codex-home="workspace.activeProfile?.codexHome"
      :profiles="workspace.profiles"
      :active-profile-id="workspace.activeProfileId"
      @select-profile="workspace.selectProfile"
      @error="notice = $event"
    />
    <div class="app-shell">
      <aside class="sidebar">
      <div class="sidebar-heading">
        <span>供应商</span>
        <span class="sidebar-actions">
          <button
            class="icon-button"
            data-testid="rediscover-providers"
            title="重新发现供应商"
            :disabled="workspace.loading"
            @click="rediscoverProviders"
          >
            <RefreshCw :size="16" :class="{ spinning: workspace.loading }" />
          </button>
          <button
            class="icon-button"
            data-testid="add-provider"
            title="添加供应商"
            :disabled="workspace.loading || workspace.providerConfigSaving"
            @click="openProviderConfigDialog('add')"
          >
            <Plus :size="16" />
          </button>
        </span>
      </div>

      <nav class="profile-list provider-list" aria-label="Provider 分组">
        <div
          v-for="provider in workspace.providerBuckets"
          :key="provider.providerId"
          class="profile-row provider-row"
          :class="{
            active: workspace.selectedProviderId === provider.providerId,
            'current-provider': provider.isCurrent,
          }"
          role="button"
          tabindex="0"
          :aria-disabled="workspace.providerSwitching"
          @click="!workspace.providerSwitching && workspace.selectProvider(provider.providerId)"
          @keydown.enter="!workspace.providerSwitching && workspace.selectProvider(provider.providerId)"
        >
          <span class="profile-icon" :class="{ current: provider.isCurrent }">
            <OpenAILogo v-if="isOfficialProvider(provider.providerId)" :size="17" />
            <Server v-else class="provider-relay-icon" :size="17" />
          </span>
          <span class="profile-copy">
            <span class="provider-name">
              <span
                class="provider-status-dot"
                :class="provider.isCurrent ? 'success' : 'info'"
                aria-hidden="true"
              />
              <strong :title="provider.providerId">{{ provider.providerId }}</strong>
            </span>
          </span>
          <span class="provider-row-meta">
            <span
              class="provider-kind-tag"
              :class="isOfficialProvider(provider.providerId) ? 'official' : 'relay'"
            >
              {{ providerKindLabel(provider.providerId) }}
            </span>
            <button
              type="button"
              class="provider-switch-button"
              :class="{ inactive: provider.isCurrent || provider.configured !== true }"
              :title="provider.isCurrent ? '当前正在使用' : provider.configured === true ? `切换到 ${provider.providerId}` : '请先在右侧完成供应商配置'"
              :aria-label="provider.isCurrent ? '当前正在使用' : provider.configured === true ? `切换到 ${provider.providerId}` : '请先在右侧完成供应商配置'"
              :disabled="provider.isCurrent || provider.configured !== true || workspace.providerConfigSwitching"
              @click.stop="requestProviderSwitch(provider.providerId)"
            >
              <ArrowRightLeft :size="13" />
            </button>
          </span>
        </div>
      </nav>
      <footer class="sidebar-version" aria-label="程序版本">
        <span class="sidebar-version-brand">by <strong>711EV</strong></span>
        <span class="sidebar-version-tools">
          <span class="sidebar-version-number">版本号 <strong>{{ appVersion }}</strong></span>
          <button
            class="sidebar-update-button"
            data-testid="check-application-update"
            :title="applicationUpdate ? '发现新版本' : '检查更新'"
            :aria-label="applicationUpdate ? '发现新版本' : '检查更新'"
            :disabled="updateChecking || updateInstalling"
            @click="checkApplicationUpdate()"
          >
            <LoaderCircle v-if="updateChecking" :size="12" class="spinning" />
            <Download v-else :size="12" />
            <span v-if="applicationUpdate" class="sidebar-update-badge" aria-hidden="true" />
          </button>
        </span>
      </footer>
      </aside>

      <main class="workspace">
      <section class="content-pane">
        <div class="pane-heading">
          <div class="pane-heading-title">
            <div class="pane-provider-cluster">
              <h2
                class="pane-provider-name"
                :title="workspace.selectedProvider?.providerId ?? '未选择 Provider'"
              >
                {{ workspace.selectedProvider?.providerId ?? "未选择 Provider" }}
              </h2>
              <span
                v-if="workspace.selectedProvider?.isCurrent"
                class="current-provider-indicator"
              >
                正在使用中
                <span class="provider-status-dot success" aria-hidden="true" />
              </span>
            </div>
          </div>
          <div class="pane-heading-actions">
            <button
              v-if="workspace.selectedProvider?.isCurrent"
              class="pane-action-button accent"
              data-testid="sync-updated-sessions"
              title="同步来源已更新和副本已更新的会话"
              :disabled="paneActionBusy"
              @click="openUpdateSyncPreview"
            >
              <LoaderCircle v-if="updateSyncPreviewLoading" :size="16" class="spinning" />
              <RefreshCcw v-else :size="16" />
              <span>同步会话</span>
            </button>
            <button
              class="pane-action-button danger"
              data-testid="cleanup-child-sessions"
              title="永久删除当前供应商中的未归档子会话"
              :disabled="paneActionBusy"
              @click="openChildCleanupPreview"
            >
              <LoaderCircle v-if="childCleanupPreviewLoading" :size="16" class="spinning" />
              <Trash2 v-else :size="16" />
              <span>清理子会话</span>
            </button>
            <button
              class="pane-action-button danger"
              data-testid="cleanup-archived-sessions"
              title="永久删除当前供应商中的已归档会话"
              :disabled="paneActionBusy"
              @click="openArchiveCleanupPreview"
            >
              <LoaderCircle v-if="archiveCleanupPreviewLoading" :size="16" class="spinning" />
              <Trash2 v-else :size="16" />
              <span>清理归档</span>
            </button>
            <button
              class="pane-action-button"
              data-testid="refresh-sessions"
              title="重新扫描供应商和会话"
              :disabled="paneActionBusy"
              @click="refreshWorkspace"
            >
              <RefreshCw :size="16" :class="{ spinning: manualRefreshing }" />
              <span>刷新会话</span>
            </button>
          </div>
        </div>

        <div class="table-tools">
          <label class="search-field">
            <Search :size="16" />
            <input v-model="workspace.search" type="search" placeholder="搜索标题、项目路径或会话 ID" />
          </label>
          <div v-if="workspace.selectedProvider" class="table-session-counts">
            <span>{{ workspace.selectedProvider.activeRootThreadCount }} 条未归档主会话</span>
            <span v-if="workspace.selectedProvider.archivedThreadCount">
              {{ workspace.selectedProvider.archivedThreadCount }} 条归档
            </span>
            <span v-if="workspace.selectedProvider.internalThreadCount">
              {{ workspace.selectedProvider.internalThreadCount }} 条子会话
            </span>
            <strong>总会话 {{ workspace.providerSessions.length }} 条</strong>
          </div>
        </div>

        <div v-if="workspace.error" class="inline-error">
          <CircleAlert :size="17" />
          <span>{{ workspace.error }}</span>
        </div>

        <div class="session-table" role="table" aria-label="Provider 会话">
          <div class="session-head" role="row">
            <label class="check-cell">
              <input
                type="checkbox"
                :checked="allVisibleSelected"
                :disabled="workspace.selectableSessions.length === 0"
                aria-label="选择所有可复制会话"
                @change="workspace.selectAllVisible"
              />
            </label>
            <span>会话</span>
            <span>来源</span>
            <span>状态</span>
            <span>更新时间</span>
            <span>大小</span>
          </div>

          <div v-if="manualRefreshing" class="empty-state">
            <LoaderCircle :size="22" class="spinning" />
            <span>正在扫描本地 Codex 会话</span>
          </div>
          <div v-else-if="sessionGroups.length === 0" class="empty-state">
            <Archive :size="23" />
            <strong>这个 Provider 没有本地会话</strong>
          </div>
          <template v-else v-for="group in sessionGroups" :key="group.session.threadId">
            <label
              class="session-row"
              :class="{
                selected: workspace.selectedThreadIds.includes(group.session.threadId),
                disabled: group.session.eligibility !== 'eligible',
                'has-children': group.children.length > 0,
              }"
              role="row"
            >
              <span class="check-cell">
                <input
                  type="checkbox"
                  :checked="workspace.selectedThreadIds.includes(group.session.threadId)"
                  :disabled="group.session.eligibility !== 'eligible'"
                  @change="workspace.toggleThread(group.session.threadId)"
                />
              </span>
              <span class="session-title-cell">
                <button
                  v-if="group.children.length"
                  type="button"
                  class="session-expand-button"
                  :title="sessionGroupExpanded(group.session.threadId) ? '收起子会话' : '展开子会话'"
                  :aria-label="sessionGroupExpanded(group.session.threadId) ? '收起子会话' : '展开子会话'"
                  :aria-expanded="sessionGroupExpanded(group.session.threadId)"
                  @click.stop.prevent="toggleSessionGroup(group.session.threadId)"
                >
                  <ChevronDown v-if="sessionGroupExpanded(group.session.threadId)" :size="15" />
                  <ChevronRight v-else :size="15" />
                </button>
                <span v-else class="session-expand-placeholder" />
                <span class="session-title-copy">
                  <strong :title="group.session.title">{{ group.session.title }}</strong>
                  <small>
                    <Folder :size="13" />
                    {{ group.session.cwd ?? group.session.threadId }}
                    <span v-if="group.children.length" class="session-child-count">
                      {{ group.children.length }} 条子会话
                    </span>
                  </small>
                </span>
              </span>
              <span class="muted-cell">{{ sessionSourceLabel(group.session) }}</span>
              <span>
                <span class="action-badge" :class="group.session.eligibility">
                  {{ eligibilityLabel(group.session.eligibility) }}
                </span>
              </span>
              <span class="muted-cell">{{ formatDate(group.session.updatedAt) }}</span>
              <span class="muted-cell">{{ formatBytes(group.session.sizeBytes) }}</span>
            </label>

            <template v-if="sessionGroupExpanded(group.session.threadId)">
              <div
                v-for="child in group.children"
                :key="child.threadId"
                class="session-row child-session-row disabled"
                role="row"
              >
                <span class="check-cell child-session-marker">
                  <GitFork :size="13" />
                </span>
                <span class="session-title-cell">
                  <span class="session-child-indent" />
                  <span class="session-title-copy">
                    <strong :title="child.title">{{ child.title }}</strong>
                    <small><Folder :size="13" /> {{ child.cwd ?? child.threadId }}</small>
                  </span>
                </span>
                <span class="muted-cell">{{ sessionSourceLabel(child) }}</span>
                <span>
                  <span class="action-badge" :class="child.eligibility">
                    {{ eligibilityLabel(child.eligibility) }}
                  </span>
                </span>
                <span class="muted-cell">{{ formatDate(child.updatedAt) }}</span>
                <span class="muted-cell">{{ formatBytes(child.sizeBytes) }}</span>
              </div>
            </template>
          </template>
        </div>
      </section>

      <aside class="sync-pane">
        <section class="sync-provider-config-section" aria-label="供应商配置">
          <div class="provider-config-heading">
            <div class="provider-config-title">
              <strong>供应商配置</strong>
            </div>
            <button
              v-if="workspace.providerConfig && workspace.providerConfig.configured && !isOfficialProvider(workspace.providerConfig.providerId)"
              class="icon-button"
              title="编辑当前供应商"
              aria-label="编辑当前供应商"
              @click="openProviderConfigDialog('edit')"
            >
              <Pencil :size="16" />
            </button>
            <button
              v-if="workspace.providerConfig?.hasPendingChanges && workspace.selectedProvider?.isCurrent"
              class="provider-apply-button"
              title="应用待保存的供应商配置"
              @click="requestApplyCurrentProvider"
            >
              <RefreshCcw :size="13" />应用
            </button>
          </div>
          <div v-if="workspace.providerConfigLoading" class="provider-config-loading">
            <LoaderCircle :size="16" class="spinning" /> 正在读取主配置
          </div>
          <div v-else-if="workspace.providerConfig && isOfficialProvider(workspace.providerConfig.providerId)" class="provider-config-card">
            <div class="provider-config-line"><span>供应商</span><strong :title="`${workspace.providerConfig.providerId}（官方）`">{{ workspace.providerConfig.providerId }}（官方）</strong></div>
            <div class="provider-config-line"><span>认证方式</span><strong>官方账号登录</strong></div>
            <div class="provider-config-line"><span>登录状态</span><strong>{{ workspace.providerConfig.authKind === 'oauth' ? '已登录' : '未登录' }}</strong></div>
            <div class="provider-config-line"><span>官方凭据快照</span><strong>{{ workspace.providerConfig.officialAuthSnapshotStatus === 'available' ? '已保存' : workspace.providerConfig.officialAuthSnapshotStatus === 'conflict' ? '完整性异常' : '未保存' }}</strong></div>
            <div v-if="workspace.providerConfig.officialAuthCapturedAt" class="provider-config-line"><span>快照时间</span><strong>{{ formatDate(workspace.providerConfig.officialAuthCapturedAt) }}</strong></div>
            <div class="provider-config-path"><span>配置文件</span><code :title="workspace.providerConfig.configFile">{{ workspace.providerConfig.configFile }}</code></div>
            <div class="provider-config-path"><span>认证文件</span><code :title="workspace.providerConfig.authFile">{{ workspace.providerConfig.authFile }}</code></div>
          </div>
          <div v-else-if="workspace.providerConfig?.configured" class="provider-config-card">
            <div class="provider-config-line"><span>供应商</span><strong :title="`${workspace.providerConfig.providerId}（中转）`">{{ workspace.providerConfig.providerId }}（中转）</strong></div>
            <div class="provider-config-line"><span>API 地址</span><strong :title="workspace.providerConfig.baseUrl ?? ''">{{ workspace.providerConfig.baseUrl ?? '未配置' }}</strong></div>
            <div class="provider-config-line"><span>API 密钥</span><strong>{{ workspace.providerConfig.apiKeyMasked ?? '未配置' }}</strong></div>
            <div class="provider-config-line"><span>认证存储</span><strong>{{ workspace.providerConfig.authStorage === 'file' ? '文件' : workspace.providerConfig.authStorage }}</strong></div>
            <div class="provider-config-path"><span>配置文件</span><code :title="workspace.providerConfig.configFile">{{ workspace.providerConfig.configFile }}</code></div>
            <div class="provider-config-path"><span>认证文件</span><code :title="workspace.providerConfig.authFile">{{ workspace.providerConfig.authFile }}</code></div>
            <div v-if="workspace.providerConfig.hasPendingChanges" class="provider-pending-state"><CircleAlert :size="14" />配置已保存，等待应用</div>
          </div>
          <div v-else class="provider-config-card provider-config-empty">
            <div class="provider-config-info-note provider-config-empty-note">
              <CircleAlert :size="18" />
              <div class="provider-config-empty-copy">
                <strong>尚未配置当前供应商</strong>
                <span>当前供应商仅存在于历史会话，或配置尚未完成。请先完成供应商配置。</span>
              </div>
            </div>
            <button class="primary-button provider-config-empty-action" @click="openProviderConfigDialog('edit')">
              <Plus :size="16" />配置供应商
            </button>
          </div>
        </section>

        <section class="sync-replication-section" aria-label="会话复制">
          <div class="replication-content">
            <div class="route-block provider-route" aria-label="会话复制路径">
              <div class="provider-route-node source">
                <span class="route-label">来源供应商</span>
                <div class="route-profile provider-route-card">
                  <span class="profile-icon compact">
                    <OpenAILogo
                      v-if="isOfficialProvider(workspace.selectedProvider?.providerId ?? '')"
                      :size="17"
                    />
                    <Server v-else class="provider-relay-icon" :size="17" />
                  </span>
                  <div>
                    <strong :title="workspace.selectedProvider?.providerId ?? ''">
                      {{ workspace.selectedProvider?.providerId ?? "未选择" }}
                    </strong>
                    <small>{{ providerKindLabel(workspace.selectedProvider?.providerId ?? "") }}</small>
                  </div>
                </div>
              </div>
              <MoveRight class="route-arrow" :size="17" aria-hidden="true" />
              <div class="provider-route-node target">
                <span class="route-label">目标供应商</span>
                <div class="route-profile provider-route-card current">
                  <span class="profile-icon compact current">
                    <OpenAILogo
                      v-if="isOfficialProvider(workspace.currentProviderId ?? '')"
                      :size="17"
                    />
                    <Server v-else class="provider-relay-icon" :size="17" />
                  </span>
                  <div>
                    <strong :title="workspace.currentProviderId ?? ''">
                      {{ workspace.currentProviderId ?? "未检测到" }}
                    </strong>
                    <small>{{ providerKindLabel(workspace.currentProviderId ?? "") }}</small>
                  </div>
                </div>
              </div>
            </div>

            <div class="replication-summary-card">
              <div class="summary-grid session-route-summary-card">
                <span>会话来源</span><strong>{{ workspace.selectedProvider?.providerId ?? "-" }}</strong>
                <span>会话目标</span><strong>{{ workspace.currentProviderId ?? "-" }}</strong>
                <span>预计新增</span><strong>{{ formatBytes(workspace.selectedSessions.reduce((sum, item) => sum + item.sizeBytes, 0)) }}</strong>
              </div>

              <div v-if="workspace.lastResult" class="last-result">
                <Check :size="16" />
                <span>上次创建 {{ workspace.lastResult.created.length }} 条，失败 {{ workspace.lastResult.failed.length }} 条</span>
              </div>

              <div class="selection-summary">
                <span>已选择</span>
                <strong>{{ workspace.selectedThreadIds.length }}</strong>
                <span>条会话</span>
              </div>
            </div>
          </div>
        </section>

        <footer class="sync-pane-footer">
          <div class="sync-pane-actions">
            <button
              class="primary-button"
              title="复制到当前供应商并保留来源会话"
              :disabled="!canPreview || previewLoading || migrationPreviewLoading"
              @click="openPreview"
            >
              <LoaderCircle v-if="previewLoading" :size="17" class="spinning" />
              <Copy v-else :size="17" />
              <span class="replication-button-label">复制</span>
            </button>
            <button
              class="secondary-button migration-trigger"
              title="迁移到当前供应商并删除来源会话"
              :disabled="!canPreview || previewLoading || migrationPreviewLoading"
              @click="openMigrationPreview"
            >
              <LoaderCircle v-if="migrationPreviewLoading" :size="17" class="spinning" />
              <MoveRight v-else :size="17" />
              <span>迁移</span>
            </button>
          </div>
        </footer>
      </aside>

      <div
        v-if="workspace.initializing || workspace.providerSwitching || workspace.providerConfigSwitching"
        class="scan-overlay"
        role="status"
        aria-live="polite"
        aria-label="正在扫描供应商会话"
      >
        <div class="scan-overlay-content">
          <LoaderCircle :size="24" class="spinning" />
          <strong>正在扫描</strong>
          <span v-if="workspace.providerConfigSwitching && workspace.selectedProviderId">
            正在应用 {{ workspace.selectedProviderId }} 的供应商配置
          </span>
          <span v-else-if="workspace.providerSwitching && workspace.selectedProviderId">
            {{ workspace.selectedProviderId }} 的本地会话
          </span>
          <span v-else>正在加载供应商和本地会话</span>
          <div class="scan-progress" aria-hidden="true"><span /></div>
        </div>
      </div>
      </main>
    </div>

    <div v-if="preview" class="modal-backdrop">
      <section class="modal preview-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">执行前检查</p><h2>会话副本预览</h2></div>
          <button class="icon-button" title="关闭" @click="preview = null"><X :size="18" /></button>
        </div>
        <div class="preview-stats">
          <div><span>创建副本</span><strong>{{ preview.createCount }}</strong></div>
          <div><span>跳过</span><strong>{{ preview.skipCount }}</strong></div>
          <div :class="{ danger: preview.invalidCount }"><span>不可复制</span><strong>{{ preview.invalidCount }}</strong></div>
          <div><span>预计新增</span><strong class="byte-stat">{{ formatBytes(preview.estimatedBytes) }}</strong></div>
        </div>
        <div class="replication-info-note">
          <CircleAlert :size="17" />
          <span>
            复制会保留 {{ workspace.selectedProvider?.providerId ?? "来源供应商" }} 中的原会话，
            并在当前供应商 {{ preview.targetProviderId }} 中创建新的独立会话；
            后续可以使用“同步会话”功能在两个会话之间同步更新。
          </span>
        </div>
        <div class="preview-list">
          <div v-for="item in preview.items" :key="item.threadId" class="preview-row">
            <span class="action-badge" :class="item.action">{{ actionLabel(item.action) }}</span>
            <div><strong>{{ item.title }}</strong><small>{{ item.reason }}</small></div>
            <span>{{ formatBytes(item.sizeBytes) }}</span>
          </div>
        </div>
        <div class="modal-actions">
          <span class="operation-note"><GitFork :size="16" />每条副本使用新的 Thread ID</span>
          <button class="secondary-button" @click="preview = null">取消</button>
          <button class="primary-button" :disabled="workspace.syncing || preview.createCount === 0" @click="runReplication(false)">
            <LoaderCircle v-if="workspace.syncing" :size="17" class="spinning" />
            <Copy v-else :size="17" />
            创建副本
          </button>
        </div>
      </section>
    </div>

    <div
      v-if="migrationPreview"
      class="modal-backdrop"
    >
      <section class="modal preview-modal migration-modal">
        <div class="modal-heading">
          <div>
            <p class="eyebrow">执行前检查</p>
            <h2>迁移会话</h2>
          </div>
          <button
            class="icon-button"
            title="关闭"
            :disabled="workspace.migrating"
            @click="closeMigrationPreview"
          >
            <X :size="18" />
          </button>
        </div>
        <div class="preview-stats">
          <div><span>迁移会话</span><strong>{{ migrationPreview.createCount }}</strong></div>
          <div><span>跳过</span><strong>{{ migrationPreview.skipCount }}</strong></div>
          <div :class="{ danger: migrationPreview.invalidCount }">
            <span>不可迁移</span><strong>{{ migrationPreview.invalidCount }}</strong>
          </div>
          <div>
            <span>预计新增</span>
            <strong class="byte-stat">{{ formatBytes(migrationPreview.estimatedBytes) }}</strong>
          </div>
        </div>
        <div class="migration-warning">
          <AlertTriangle :size="17" />
          <span>
            迁移成功后，{{ workspace.selectedProvider?.providerId ?? "来源供应商" }} 中的原会话将被永久删除且不会备份；
            新会话会归入 {{ migrationPreview.targetProviderId }} 并显示为“当前”。
          </span>
        </div>
        <div class="target-disclosure migration-route">
          <span>迁移路径</span>
          <strong>
            {{ workspace.selectedProvider?.providerId ?? "-" }} → {{ migrationPreview.targetProviderId }}
          </strong>
          <small>{{ workspace.activeProfile?.codexHome }}</small>
        </div>
        <div class="preview-list">
          <div v-for="item in migrationPreview.items" :key="item.threadId" class="preview-row">
            <span class="action-badge" :class="item.action">{{ actionLabel(item.action) }}</span>
            <div><strong>{{ item.title }}</strong><small>{{ item.reason }}</small></div>
            <span>{{ formatBytes(item.sizeBytes) }}</span>
          </div>
        </div>
        <div class="modal-actions">
          <span class="operation-note"><GitFork :size="16" />迁移后会生成新的 Thread ID</span>
          <button class="secondary-button" :disabled="workspace.migrating" @click="closeMigrationPreview">
            取消
          </button>
          <button
            class="danger-button"
            :disabled="workspace.migrating || migrationPreview.createCount === 0"
            @click="runMigration(false)"
          >
            <LoaderCircle v-if="workspace.migrating" :size="17" class="spinning" />
            <MoveRight v-else :size="17" />
            确认迁移 {{ migrationPreview.createCount }} 条
          </button>
        </div>
      </section>
    </div>

    <div
      v-if="updateSyncPreview"
      class="modal-backdrop"
    >
      <section class="modal preview-modal update-sync-modal">
        <div class="modal-heading">
          <div>
            <p class="eyebrow">当前供应商 {{ updateSyncPreview.targetProviderId }}</p>
            <h2>{{ updateSyncResult ? "同步结果" : "同步会话" }}</h2>
          </div>
          <button
            class="icon-button"
            title="关闭"
            :disabled="workspace.syncing"
            @click="closeUpdateSyncPreview"
          >
            <X :size="18" />
          </button>
        </div>

        <template v-if="!updateSyncResult">
          <div class="preview-stats update-sync-stats">
            <div><span>需要同步</span><strong>{{ updateSyncPreview.updateCount }}</strong></div>
            <div :class="{ danger: updateSyncPreview.conflictCount }">
              <span>冲突</span><strong>{{ updateSyncPreview.conflictCount }}</strong>
            </div>
          </div>
          <div v-if="updateSyncPreview.items.length" class="preview-list update-sync-list">
            <div
              v-for="item in updateSyncPreview.items"
              :key="item.mappingId"
              class="preview-row update-sync-row"
            >
              <span class="action-badge" :class="item.action">
                {{ updateSyncActionLabel(item.action) }}
              </span>
              <div>
                <strong>{{ item.title }}</strong>
                <small>{{ updateSyncRoute(item) }} · {{ item.reason }}</small>
              </div>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>当前没有需要同步的会话</strong>
          </div>
          <div class="modal-actions">
            <button class="secondary-button" :disabled="workspace.syncing" @click="closeUpdateSyncPreview">
              取消
            </button>
            <button
              class="primary-button"
              :disabled="workspace.syncing || updateSyncPreview.updateCount === 0"
              @click="runUpdateSync(false)"
            >
              <LoaderCircle v-if="workspace.syncing" :size="17" class="spinning" />
              <RefreshCcw v-else :size="17" />
              开始同步 {{ updateSyncPreview.updateCount }} 条
            </button>
          </div>
        </template>

        <template v-else>
          <div class="preview-stats update-result-stats">
            <div><span>已更新</span><strong>{{ updateSyncResult.created.length }}</strong></div>
            <div><span>已跳过</span><strong>{{ updateSyncResult.skipped.length }}</strong></div>
            <div :class="{ danger: updateSyncResult.failed.length }">
              <span>失败</span><strong>{{ updateSyncResult.failed.length }}</strong>
            </div>
          </div>
          <div
            v-if="updateSyncResult.created.length || updateSyncResult.skipped.length || updateSyncResult.failed.length"
            class="preview-list update-sync-list"
          >
            <div
              v-for="item in updateSyncResult.created"
              :key="`created-${item.sourceThreadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge synchronized">已更新</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
            <div
              v-for="item in updateSyncResult.skipped"
              :key="`skipped-${item.sourceThreadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge conflict">已跳过</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
            <div
              v-for="item in updateSyncResult.failed"
              :key="`failed-${item.sourceThreadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge invalid">失败</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>当前没有需要同步的会话</strong>
          </div>
          <div class="modal-actions">
            <span v-if="updateSyncResult.warning" class="operation-note">
              <CircleAlert :size="16" />{{ updateSyncResult.warning }}
            </span>
            <button class="primary-button" @click="closeUpdateSyncPreview">完成</button>
          </div>
        </template>
      </section>
    </div>

    <div
      v-if="childCleanupPreview"
      class="modal-backdrop"
    >
      <section class="modal preview-modal child-cleanup-modal">
        <div class="modal-heading">
          <div>
            <p class="eyebrow">当前供应商 {{ childCleanupPreview.providerId }}</p>
            <h2>{{ childCleanupResult ? "清理结果" : "清理子会话" }}</h2>
          </div>
          <button
            class="icon-button"
            title="关闭"
            :disabled="workspace.cleaningChildren"
            @click="closeChildCleanupPreview"
          >
            <X :size="18" />
          </button>
        </div>

        <template v-if="!childCleanupResult">
          <div class="preview-stats child-cleanup-stats">
            <div :class="{ danger: childCleanupPreview.totalCount }">
              <span>未归档子会话</span><strong>{{ childCleanupPreview.totalCount }}</strong>
            </div>
            <div>
              <span>占用空间</span><strong class="byte-stat">{{ formatBytes(childCleanupPreview.totalBytes) }}</strong>
            </div>
          </div>
          <div class="child-cleanup-warning">
            <AlertTriangle :size="17" />
            <span>这些子会话将被永久删除且不会备份，删除后无法通过本工具恢复。</span>
          </div>
          <div class="child-cleanup-impact-note">
            <CircleAlert :size="17" />
            <span>删除子会话不会影响其所属主会话，主会话内容和后续使用保持不变。</span>
          </div>
          <div v-if="childCleanupPreview.items.length" class="preview-list child-cleanup-list">
            <div
              v-for="item in childCleanupPreview.items"
              :key="item.threadId"
              class="preview-row child-cleanup-row"
            >
              <span class="action-badge internal_thread">子会话</span>
              <div>
                <strong>{{ item.title }}</strong>
                <small>{{ sourceLabel(item.sourceKind) }} · {{ formatDate(item.updatedAt) }}</small>
              </div>
              <span>{{ formatBytes(item.sizeBytes) }}</span>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>当前供应商没有可清理的未归档子会话</strong>
          </div>
          <div class="modal-actions">
            <button
              class="secondary-button"
              :disabled="workspace.cleaningChildren"
              @click="closeChildCleanupPreview"
            >
              取消
            </button>
            <button
              class="danger-button"
              :disabled="workspace.cleaningChildren || childCleanupPreview.totalCount === 0"
              @click="runChildCleanup(false)"
            >
              <LoaderCircle v-if="workspace.cleaningChildren" :size="17" class="spinning" />
              <Trash2 v-else :size="17" />
              永久删除 {{ childCleanupPreview.totalCount }} 条
            </button>
          </div>
        </template>

        <template v-else>
          <div class="preview-stats child-cleanup-stats">
            <div><span>已删除</span><strong>{{ childCleanupResult.deleted.length }}</strong></div>
            <div :class="{ danger: childCleanupResult.failed.length }">
              <span>失败</span><strong>{{ childCleanupResult.failed.length }}</strong>
            </div>
          </div>
          <div
            v-if="childCleanupResult.deleted.length || childCleanupResult.failed.length"
            class="preview-list child-cleanup-list"
          >
            <div
              v-for="item in childCleanupResult.deleted"
              :key="`child-deleted-${item.threadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge deleted">已删除</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
            <div
              v-for="item in childCleanupResult.failed"
              :key="`child-failed-${item.threadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge invalid">失败</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>没有执行任何删除操作</strong>
          </div>
          <div class="modal-actions">
            <span v-if="childCleanupResult.warning" class="operation-note">
              <CircleAlert :size="16" />{{ childCleanupResult.warning }}
            </span>
            <button class="primary-button" @click="closeChildCleanupPreview">完成</button>
          </div>
        </template>
      </section>
    </div>

    <div
      v-if="archiveCleanupPreview"
      class="modal-backdrop"
    >
      <section class="modal preview-modal archive-cleanup-modal">
        <div class="modal-heading">
          <div>
            <p class="eyebrow">当前供应商 {{ archiveCleanupPreview.providerId }}</p>
            <h2>{{ archiveCleanupResult ? "清理结果" : "清理归档" }}</h2>
          </div>
          <button
            class="icon-button"
            title="关闭"
            :disabled="workspace.cleaningArchived"
            @click="closeArchiveCleanupPreview"
          >
            <X :size="18" />
          </button>
        </div>

        <template v-if="!archiveCleanupResult">
          <div class="preview-stats archive-cleanup-stats">
            <div :class="{ danger: archiveCleanupPreview.totalCount }">
              <span>归档会话</span><strong>{{ archiveCleanupPreview.totalCount }}</strong>
            </div>
            <div>
              <span>占用空间</span><strong class="byte-stat">{{ formatBytes(archiveCleanupPreview.totalBytes) }}</strong>
            </div>
          </div>
          <div class="archive-cleanup-warning">
            <AlertTriangle :size="17" />
            <span>这些会话将被永久删除且不会备份，删除后无法通过本工具恢复。</span>
          </div>
          <div v-if="archiveCleanupPreview.items.length" class="preview-list archive-cleanup-list">
            <div
              v-for="item in archiveCleanupPreview.items"
              :key="item.threadId"
              class="preview-row archive-cleanup-row"
            >
              <span class="action-badge archived">已归档</span>
              <div>
                <strong>{{ item.title }}</strong>
                <small>{{ sourceLabel(item.sourceKind) }} · {{ formatDate(item.updatedAt) }}</small>
              </div>
              <span>{{ formatBytes(item.sizeBytes) }}</span>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>当前供应商没有已归档会话</strong>
          </div>
          <div class="modal-actions">
            <button
              class="secondary-button"
              :disabled="workspace.cleaningArchived"
              @click="closeArchiveCleanupPreview"
            >
              取消
            </button>
            <button
              class="danger-button"
              :disabled="workspace.cleaningArchived || archiveCleanupPreview.totalCount === 0"
              @click="runArchiveCleanup(false)"
            >
              <LoaderCircle v-if="workspace.cleaningArchived" :size="17" class="spinning" />
              <Trash2 v-else :size="17" />
              永久删除 {{ archiveCleanupPreview.totalCount }} 条
            </button>
          </div>
        </template>

        <template v-else>
          <div class="preview-stats archive-cleanup-stats">
            <div><span>已删除</span><strong>{{ archiveCleanupResult.deleted.length }}</strong></div>
            <div :class="{ danger: archiveCleanupResult.failed.length }">
              <span>失败</span><strong>{{ archiveCleanupResult.failed.length }}</strong>
            </div>
          </div>
          <div
            v-if="archiveCleanupResult.deleted.length || archiveCleanupResult.failed.length"
            class="preview-list archive-cleanup-list"
          >
            <div
              v-for="item in archiveCleanupResult.deleted"
              :key="`deleted-${item.threadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge deleted">已删除</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
            <div
              v-for="item in archiveCleanupResult.failed"
              :key="`failed-${item.threadId}`"
              class="preview-row update-sync-row"
            >
              <span class="action-badge invalid">失败</span>
              <div><strong>{{ item.title }}</strong><small>{{ item.message }}</small></div>
            </div>
          </div>
          <div v-else class="empty-state compact-empty-state">
            <Check :size="22" />
            <strong>没有执行任何删除操作</strong>
          </div>
          <div class="modal-actions">
            <span v-if="archiveCleanupResult.warning" class="operation-note">
              <CircleAlert :size="16" />{{ archiveCleanupResult.warning }}
            </span>
            <button class="primary-button" @click="closeArchiveCleanupPreview">完成</button>
          </div>
        </template>
      </section>
    </div>

    <div v-if="providerConfigDialog" class="modal-backdrop">
      <section class="modal provider-config-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">供应商配置</p><h2>{{ providerConfigDialog === 'add' ? '添加供应商' : '编辑供应商' }}</h2></div>
          <button class="icon-button" title="关闭" @click="closeProviderConfigDialog"><X :size="18" /></button>
        </div>
        <div class="provider-config-form">
          <div v-if="providerConfigDialog === 'add'" class="provider-template-option">
            <strong>使用711EV配置</strong>
            <button
              type="button"
              class="provider-toggle-switch"
              :class="{ active: providerConfigIs711Ev }"
              role="switch"
              :aria-checked="providerConfigIs711Ev"
              :title="providerConfigIs711Ev ? '关闭711EV预设' : '使用711EV预设'"
              aria-label="使用711EV配置"
              @click="toggle711EvProvider"
            ><span /></button>
          </div>
          <div class="provider-config-fields">
            <label class="provider-config-field">
              <span>供应商</span>
              <input v-model="providerConfigDraft.providerId" :disabled="providerConfigDialog === 'edit'" autocomplete="off" placeholder="示例 711EV-Codex" />
            </label>
            <label class="provider-config-field full-field">
              <span>API 请求地址</span>
              <input v-model="providerConfigDraft.baseUrl" placeholder="示例 https://ai.711ev.com/v1" />
            </label>
            <label class="provider-config-field full-field">
              <span>API 密钥</span>
              <span class="provider-secret-control">
                <input v-model="providerConfigDraft.apiKey" :type="providerConfigKeyVisible ? 'text' : 'password'" autocomplete="new-password" placeholder="示例 sk-xxxxxxxxxxxxxxxxxxx" />
                <button
                  type="button"
                  class="provider-secret-toggle"
                  :title="providerConfigKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'"
                  :aria-label="providerConfigKeyVisible ? '隐藏 API 密钥' : '显示 API 密钥'"
                  @click="providerConfigKeyVisible = !providerConfigKeyVisible"
                >
                  <EyeOff v-if="providerConfigKeyVisible" :size="16" />
                  <Eye v-else :size="16" />
                </button>
              </span>
            </label>
          </div>
          <div class="provider-save-preview">
            <div><span>配置文件</span><code :title="workspace.providerConfig?.configFile ?? `${workspace.activeProfile?.codexHome ?? ''}/config.toml`">{{ workspace.providerConfig?.configFile ?? `${workspace.activeProfile?.codexHome ?? ''}/config.toml` }}</code></div>
            <div><span>密钥存储</span><code>本地数据库 SQLite</code></div>
          </div>
        </div>
        <div class="modal-actions">
          <button class="secondary-button" @click="closeProviderConfigDialog">取消</button>
          <button class="primary-button" :disabled="workspace.providerConfigSaving" @click="saveProviderConfigDialog">
            <LoaderCircle v-if="workspace.providerConfigSaving" :size="16" class="spinning" />
            <Check v-else :size="16" />保存供应商
          </button>
        </div>
      </section>
    </div>

    <div v-if="providerSwitchDialog" class="modal-backdrop high-priority">
      <section class="modal provider-switch-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">切换供应商</p><h2>切换到 {{ providerSwitchDialog }}</h2></div>
          <button class="icon-button" title="关闭" :disabled="workspace.providerConfigSwitching" @click="closeProviderSwitchDialog"><X :size="18" /></button>
        </div>
        <div class="replication-info-note">
          <CircleAlert :size="17" />
          <span>工具只修改当前存储位置的主配置文件，不会修改 profile 子配置。</span>
        </div>
        <div v-if="providerSwitchDialog.toLocaleLowerCase() === 'openai' && workspace.providerConfig?.officialAuthSnapshotStatus !== 'available'" class="migration-warning">
          <AlertTriangle :size="17" /><span>未发现官方快照，将删除当前 auth.json，后续需要重新登录。</span>
        </div>
        <div class="target-disclosure provider-switch-paths">
          <span>配置文件</span>
          <strong :title="workspace.providerConfig?.configFile ?? `${workspace.activeProfile?.codexHome ?? ''}/config.toml`">{{ workspace.providerConfig?.configFile ?? `${workspace.activeProfile?.codexHome ?? ''}/config.toml` }}</strong>
          <span>认证文件</span>
          <strong :title="workspace.providerConfig?.authFile ?? `${workspace.activeProfile?.codexHome ?? ''}/auth.json`">{{ workspace.providerConfig?.authFile ?? `${workspace.activeProfile?.codexHome ?? ''}/auth.json` }}</strong>
        </div>
        <div class="modal-actions">
          <button class="secondary-button" @click="closeProviderSwitchDialog">取消</button>
          <button class="primary-button" :disabled="workspace.providerConfigSwitching" @click="confirmProviderSwitch">
            <LoaderCircle v-if="workspace.providerConfigSwitching" :size="16" class="spinning" />
            <ArrowRightLeft v-else :size="16" />确认切换
          </button>
        </div>
      </section>
    </div>

    <div v-if="providerSwitchResult" class="modal-backdrop high-priority">
      <section class="modal restart-client-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">供应商切换完成</p><h2>是否重启 Codex Desktop？</h2></div>
          <button class="icon-button" title="关闭" :disabled="providerSwitchRestarting" @click="dismissProviderSwitchRestart"><X :size="18" /></button>
        </div>
        <p class="restart-client-summary">配置已经写入磁盘。重启 Desktop 后它才会读取新的供应商和认证状态；CLI 下次运行时会读取最新配置。</p>
        <div class="restart-client-warning"><AlertTriangle :size="17" /><span>工具不会判断当前是否有运行任务，请根据实际情况决定是否现在重启。</span></div>
        <div class="modal-actions">
          <button class="secondary-button" :disabled="providerSwitchRestarting" @click="dismissProviderSwitchRestart">暂不重启</button>
          <button class="primary-button" :disabled="providerSwitchRestarting" @click="restartAfterProviderSwitch(false)">
            <LoaderCircle v-if="providerSwitchRestarting" :size="16" class="spinning" /><RefreshCw v-else :size="16" />重启 Desktop
          </button>
        </div>
      </section>
    </div>

    <div v-if="forceClosePrompt" class="modal-backdrop high-priority">
      <section class="modal force-close-modal">
        <div class="modal-heading">
          <div><p class="eyebrow danger-text">操作被阻止</p><h2>Codex 客户端未能正常退出</h2></div>
          <button class="icon-button" title="关闭" @click="closeForcePrompt"><X :size="18" /></button>
        </div>
        <div class="force-close-warning"><AlertTriangle :size="17" /><span>强制结束可能中断正在执行的任务或尚未落盘的数据。只会处理与当前 CODEX_HOME 匹配的客户端进程。</span></div>
        <div class="modal-actions">
          <button class="secondary-button" @click="closeForcePrompt">
            {{ pendingForceOperation === "archive-cleanup" || pendingForceOperation === "child-cleanup" ? "取消清理" : pendingForceOperation === "update-sync" ? "取消同步" : pendingForceOperation === "client-restart" ? "取消重启" : pendingForceOperation === "migration" ? "取消迁移" : "取消复制" }}
          </button>
          <button class="danger-button" :disabled="workspace.syncing || workspace.migrating || workspace.restartingClient || workspace.cleaningArchived || workspace.cleaningChildren" @click="retryForcedOperation">
            <Trash2 :size="17" />强制结束并继续
          </button>
        </div>
      </section>
    </div>

    <div
      v-if="restartPrompt"
      class="modal-backdrop"
    >
      <section class="modal restart-client-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">{{ restartPrompt.operation }}完成</p><h2>是否重启 Codex Desktop？</h2></div>
          <button class="icon-button" title="关闭" :disabled="workspace.restartingClient" @click="dismissRestartPrompt"><X :size="18" /></button>
        </div>
        <p class="restart-client-summary">
          已{{ restartPrompt.operation }} {{ restartPrompt.completedCount }} 条会话。重启后，新会话才会显示在 Codex Desktop 的会话列表中。
        </p>
        <div class="restart-client-warning">
          <AlertTriangle :size="17" />
          <span>重启会关闭当前 Desktop 客户端。如果有正在运行的任务，请选择“暂不重启”，任务完成后再手动重启。</span>
        </div>
        <div class="restart-cli-note">
          <CircleAlert :size="17" />
          <span>使用 CLI 无需重启；下次运行 codex resume 时会重新读取会话列表。</span>
        </div>
        <div class="modal-actions">
          <button
            class="secondary-button"
            :disabled="workspace.restartingClient"
            @click="dismissRestartPrompt"
          >
            暂不重启
          </button>
          <button
            class="primary-button"
            :disabled="workspace.restartingClient"
            @click="restartCodexDesktop(false)"
          >
            <LoaderCircle v-if="workspace.restartingClient" :size="17" class="spinning" />
            <RefreshCw v-else :size="17" />
            重启 Desktop
          </button>
        </div>
      </section>
    </div>

    <div
      v-if="applicationUpdate && updateDialogOpen"
      class="modal-backdrop high-priority"
    >
      <section class="modal application-update-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">发现新版本</p><h2>711EV-Codex-Tool {{ applicationUpdate.version }}</h2></div>
          <button class="icon-button" title="关闭" :disabled="updateInstalling" @click="closeApplicationUpdate"><X :size="18" /></button>
        </div>
        <div class="application-update-versions">
          <span>当前版本 <strong>{{ applicationUpdate.currentVersion }}</strong></span>
          <MoveRight :size="16" />
          <span>最新版本 <strong>{{ applicationUpdate.version }}</strong></span>
        </div>
        <p v-if="applicationUpdate.body" class="application-update-notes">
          {{ applicationUpdate.body }}
        </p>
        <div class="application-update-warning">
          <CircleAlert :size="17" />
          <span>升级包通过签名验证后安装，安装完成会自动重启本工具，现有会话关联数据不会被覆盖。</span>
        </div>
        <div v-if="updateInstalling" class="application-update-progress">
          <div><span :style="{ width: `${updateProgress?.percent ?? 12}%` }" /></div>
          <small>
            {{ updateProgress?.percent == null ? "正在下载升级包" : `已下载 ${updateProgress.percent}%` }}
          </small>
        </div>
        <div class="modal-actions">
          <button
            class="secondary-button"
            :disabled="updateInstalling"
            @click="closeApplicationUpdate"
          >
            稍后升级
          </button>
          <button
            class="primary-button"
            :disabled="updateInstalling"
            @click="installApplicationUpdate"
          >
            <LoaderCircle v-if="updateInstalling" :size="17" class="spinning" />
            <Download v-else :size="17" />
            {{ updateInstalling ? "正在升级" : "立即升级" }}
          </button>
        </div>
      </section>
    </div>

    <div v-if="notice" class="toast">
      <span>{{ notice }}</span>
      <button class="icon-button" title="关闭" @click="notice = null"><X :size="16" /></button>
    </div>
  </div>
</template>
