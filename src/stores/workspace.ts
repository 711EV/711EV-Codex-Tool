import { computed, nextTick, ref } from "vue";
import { defineStore } from "pinia";
import { backend } from "../services/backend";
import type {
  AppState,
  ArchiveCleanupPreview,
  InvalidChildCleanupPreview,
  ProfileInput,
  ProviderBucket,
  ProviderSessionRecord,
  ProviderWorkspaceSnapshot,
  ReplicationPreview,
  ReplicationResult,
  UpdateSyncPreview,
} from "../types";

const PROVIDER_SWITCH_MIN_MS = 500;

export const useWorkspaceStore = defineStore("workspace", () => {
  const appState = ref<AppState | null>(null);
  const providerBuckets = ref<ProviderBucket[]>([]);
  const providerSessions = ref<ProviderSessionRecord[]>([]);
  const activeProfileId = ref<string | null>(null);
  const selectedProviderId = ref<string | null>(null);
  const selectedThreadIds = ref<string[]>([]);
  const search = ref("");
  const initializing = ref(true);
  const loading = ref(false);
  const providerSwitching = ref(false);
  const syncing = ref(false);
  const migrating = ref(false);
  const restartingClient = ref(false);
  const cleaningArchived = ref(false);
  const cleaningChildren = ref(false);
  const discovering = ref(false);
  const error = ref<string | null>(null);
  const lastResult = ref<ReplicationResult | null>(null);

  const profiles = computed(() => appState.value?.profiles ?? []);
  const activeProfile = computed(() =>
    profiles.value.find((profile) => profile.id === activeProfileId.value),
  );
  const selectedProvider = computed(() =>
    providerBuckets.value.find((provider) => provider.providerId === selectedProviderId.value),
  );
  const currentProvider = computed(() =>
    providerBuckets.value.find((provider) => provider.isCurrent),
  );
  const currentProviderId = computed(() => currentProvider.value?.providerId ?? null);
  const filteredSessions = computed(() => {
    const term = search.value.trim().toLocaleLowerCase();
    if (!term) return providerSessions.value;
    return providerSessions.value.filter((session) =>
      [session.title, session.cwd, session.threadId, session.eligibilityReason]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(term)),
    );
  });
  const selectedSessions = computed(() => {
    const selected = new Set(selectedThreadIds.value);
    return providerSessions.value.filter((session) => selected.has(session.threadId));
  });
  const selectableSessions = computed(() =>
    filteredSessions.value.filter((session) => session.eligibility === "eligible"),
  );

  async function initialize() {
    initializing.value = true;
    loading.value = true;
    error.value = null;
    try {
      appState.value = await backend.getAppState();
      activeProfileId.value ||= profiles.value[0]?.id ?? null;
      await refreshProviders();
    } catch (reason) {
      error.value = messageOf(reason);
    } finally {
      loading.value = false;
      initializing.value = false;
    }
  }

  async function refreshProviders() {
    if (!activeProfileId.value) {
      providerBuckets.value = [];
      providerSessions.value = [];
      selectedProviderId.value = null;
      return;
    }
    loading.value = true;
    error.value = null;
    try {
      applyProviderWorkspace(await backend.providerWorkspace(
        activeProfileId.value,
        selectedProviderId.value,
      ));
    } catch (reason) {
      error.value = messageOf(reason);
    } finally {
      loading.value = false;
    }
  }

  async function refreshSessions(showLoading = true) {
    if (!activeProfileId.value || !selectedProviderId.value) {
      providerSessions.value = [];
      selectedThreadIds.value = [];
      return;
    }
    if (showLoading) loading.value = true;
    error.value = null;
    try {
      applyProviderWorkspace(await backend.providerWorkspace(
        activeProfileId.value,
        selectedProviderId.value,
      ));
    } catch (reason) {
      error.value = messageOf(reason);
    } finally {
      if (showLoading) loading.value = false;
    }
  }

  function applyProviderWorkspace(snapshot: ProviderWorkspaceSnapshot) {
    providerBuckets.value = snapshot.providerBuckets;
    selectedProviderId.value = snapshot.selectedProviderId;
    providerSessions.value = snapshot.providerSessions;
    const available = new Set(
      providerSessions.value
        .filter((session) => session.eligibility === "eligible")
        .map((session) => session.threadId),
    );
    selectedThreadIds.value = selectedThreadIds.value.filter((id) => available.has(id));
  }

  async function selectProfile(profileId: string) {
    activeProfileId.value = profileId;
    selectedProviderId.value = null;
    selectedThreadIds.value = [];
    await refreshProviders();
  }

  async function selectProvider(providerId: string) {
    if (providerId === selectedProviderId.value || providerSwitching.value) return;

    const startedAt = Date.now();
    providerSwitching.value = true;
    selectedProviderId.value = providerId;
    selectedThreadIds.value = [];
    try {
      await nextTick();
      if (typeof requestAnimationFrame === "function") {
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      }
      await refreshSessions();
    } finally {
      const remaining = PROVIDER_SWITCH_MIN_MS - (Date.now() - startedAt);
      if (remaining > 0) {
        await new Promise<void>((resolve) => window.setTimeout(resolve, remaining));
      }
      providerSwitching.value = false;
    }
  }

  function toggleThread(threadId: string) {
    const session = providerSessions.value.find((item) => item.threadId === threadId);
    if (!session || session.eligibility !== "eligible") return;
    selectedThreadIds.value = selectedThreadIds.value.includes(threadId)
      ? selectedThreadIds.value.filter((id) => id !== threadId)
      : [...selectedThreadIds.value, threadId];
  }

  function selectAllVisible() {
    const visible = selectableSessions.value.map((session) => session.threadId);
    const selected = new Set(selectedThreadIds.value);
    const allSelected = visible.length > 0 && visible.every((id) => selected.has(id));
    selectedThreadIds.value = allSelected
      ? selectedThreadIds.value.filter((id) => !visible.includes(id))
      : [...new Set([...selectedThreadIds.value, ...visible])];
  }

  async function createProfile(input: ProfileInput) {
    const profile = await backend.createProfile(input);
    appState.value = await backend.getAppState();
    await selectProfile(profile.id);
  }

  async function deleteProfile(profileId: string) {
    await backend.deleteProfile(profileId);
    appState.value = await backend.getAppState();
    if (activeProfileId.value === profileId) {
      activeProfileId.value = profiles.value[0]?.id ?? null;
    }
    selectedProviderId.value = null;
    await refreshProviders();
  }

  async function discoverProfiles() {
    discovering.value = true;
    error.value = null;
    try {
      const report = await backend.discoverProfiles();
      if (appState.value) appState.value.profiles = report.profiles;
      else appState.value = await backend.getAppState();
      if (!profiles.value.some((profile) => profile.id === activeProfileId.value)) {
        activeProfileId.value = profiles.value[0]?.id ?? null;
      }
      await refreshProviders();
      return report;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      discovering.value = false;
    }
  }

  async function previewReplication(): Promise<ReplicationPreview> {
    if (!activeProfileId.value || selectedThreadIds.value.length === 0) {
      throw new Error("请至少选择一条可复制会话");
    }
    return backend.replicationPreview(activeProfileId.value, selectedThreadIds.value);
  }

  async function previewArchivedCleanup(): Promise<ArchiveCleanupPreview> {
    if (!activeProfileId.value || !selectedProviderId.value) {
      throw new Error("未选择需要清理的供应商");
    }
    return backend.archiveCleanupPreview(activeProfileId.value, selectedProviderId.value);
  }

  async function cleanupArchivedSessions(
    providerId: string,
    threadIds: string[],
    forceCloseClient = false,
  ) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    cleaningArchived.value = true;
    error.value = null;
    try {
      const result = await backend.archiveCleanupExecute(
        activeProfileId.value,
        providerId,
        threadIds,
        forceCloseClient,
      );
      await refreshProviders();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      cleaningArchived.value = false;
    }
  }

  async function previewInvalidChildCleanup(): Promise<InvalidChildCleanupPreview> {
    if (!activeProfileId.value || !selectedProviderId.value) {
      throw new Error("未选择需要清理的供应商");
    }
    return backend.invalidChildCleanupPreview(
      activeProfileId.value,
      selectedProviderId.value,
    );
  }

  async function cleanupInvalidChildSessions(
    providerId: string,
    threadIds: string[],
    forceCloseClient = false,
  ) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    cleaningChildren.value = true;
    error.value = null;
    try {
      const result = await backend.invalidChildCleanupExecute(
        activeProfileId.value,
        providerId,
        threadIds,
        forceCloseClient,
      );
      await refreshProviders();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      cleaningChildren.value = false;
    }
  }

  async function executeReplication(forceCloseClient = false) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    syncing.value = true;
    error.value = null;
    try {
      const result = await backend.replicationExecute(
        activeProfileId.value,
        selectedThreadIds.value,
        forceCloseClient,
      );
      lastResult.value = result;
      selectedThreadIds.value = [];
      await refreshProviders();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      syncing.value = false;
    }
  }

  async function executeMigration(forceCloseClient = false) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    migrating.value = true;
    error.value = null;
    try {
      const result = await backend.replicationMigrate(
        activeProfileId.value,
        selectedThreadIds.value,
        forceCloseClient,
      );
      lastResult.value = result;
      selectedThreadIds.value = [];
      await refreshProviders();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      migrating.value = false;
    }
  }

  async function restartCodexClient(forceCloseClient = false) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    restartingClient.value = true;
    error.value = null;
    try {
      return await backend.restartCodexClient(activeProfileId.value, forceCloseClient);
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      restartingClient.value = false;
    }
  }

  async function syncUpdatedSessions(forceCloseClient = false) {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    if (!selectedProvider.value?.isCurrent) {
      throw new Error("同步会话只能在当前供应商中执行");
    }
    syncing.value = true;
    error.value = null;
    try {
      const result = await backend.replicationSyncUpdates(
        activeProfileId.value,
        forceCloseClient,
      );
      lastResult.value = result;
      await refreshProviders();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      syncing.value = false;
    }
  }

  async function previewUpdatedSessions(): Promise<UpdateSyncPreview> {
    if (!activeProfileId.value) throw new Error("未选择 CODEX_HOME");
    if (!selectedProvider.value?.isCurrent) {
      throw new Error("同步会话只能在当前供应商中执行");
    }
    return backend.replicationSyncPreview(activeProfileId.value);
  }

  return {
    appState,
    providerBuckets,
    providerSessions,
    activeProfileId,
    selectedProviderId,
    selectedThreadIds,
    search,
    initializing,
    loading,
    providerSwitching,
    syncing,
    migrating,
    restartingClient,
    cleaningArchived,
    cleaningChildren,
    discovering,
    error,
    lastResult,
    profiles,
    activeProfile,
    selectedProvider,
    currentProvider,
    currentProviderId,
    filteredSessions,
    selectedSessions,
    selectableSessions,
    initialize,
    refreshProviders,
    refreshSessions,
    selectProfile,
    selectProvider,
    toggleThread,
    selectAllVisible,
    createProfile,
    deleteProfile,
    discoverProfiles,
    previewArchivedCleanup,
    cleanupArchivedSessions,
    previewInvalidChildCleanup,
    cleanupInvalidChildSessions,
    previewReplication,
    executeReplication,
    executeMigration,
    restartCodexClient,
    previewUpdatedSessions,
    syncUpdatedSessions,
  };
});

function messageOf(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}
