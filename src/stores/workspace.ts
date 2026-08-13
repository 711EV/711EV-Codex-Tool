import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { backend } from "../services/backend";
import type {
  AppState,
  ProfileInput,
  SessionRecord,
  SyncPreview,
  SyncResult,
} from "../types";

export const useWorkspaceStore = defineStore("workspace", () => {
  const appState = ref<AppState | null>(null);
  const sessions = ref<SessionRecord[]>([]);
  const activeProfileId = ref<string | null>(null);
  const targetProfileId = ref<string | null>(null);
  const selectedThreadIds = ref<string[]>([]);
  const search = ref("");
  const loading = ref(false);
  const syncing = ref(false);
  const error = ref<string | null>(null);
  const lastResult = ref<SyncResult | null>(null);

  const profiles = computed(() => appState.value?.profiles ?? []);
  const activeProfile = computed(() =>
    profiles.value.find((profile) => profile.id === activeProfileId.value),
  );
  const targetProfile = computed(() =>
    profiles.value.find((profile) => profile.id === targetProfileId.value),
  );
  const filteredSessions = computed(() => {
    const term = search.value.trim().toLocaleLowerCase();
    if (!term) return sessions.value;
    return sessions.value.filter((session) =>
      [session.title, session.cwd, session.threadId]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(term)),
    );
  });
  const selectedSessions = computed(() => {
    const selected = new Set(selectedThreadIds.value);
    return sessions.value.filter((session) => selected.has(session.threadId));
  });

  async function initialize() {
    loading.value = true;
    error.value = null;
    try {
      appState.value = await backend.getAppState();
      activeProfileId.value ||= profiles.value[0]?.id ?? null;
      setDefaultTarget();
      await refreshSessions();
    } catch (reason) {
      error.value = messageOf(reason);
    } finally {
      loading.value = false;
    }
  }

  async function refreshSessions() {
    if (!activeProfileId.value) {
      sessions.value = [];
      selectedThreadIds.value = [];
      return;
    }
    loading.value = true;
    error.value = null;
    try {
      sessions.value = await backend.scanSessions(activeProfileId.value);
      const available = new Set(sessions.value.map((session) => session.threadId));
      selectedThreadIds.value = selectedThreadIds.value.filter((id) => available.has(id));
    } catch (reason) {
      error.value = messageOf(reason);
    } finally {
      loading.value = false;
    }
  }

  async function selectProfile(profileId: string) {
    activeProfileId.value = profileId;
    selectedThreadIds.value = [];
    if (targetProfileId.value === profileId) setDefaultTarget();
    await refreshSessions();
  }

  function setDefaultTarget() {
    if (!targetProfileId.value || targetProfileId.value === activeProfileId.value) {
      targetProfileId.value = profiles.value.find((profile) => profile.id !== activeProfileId.value)?.id ?? null;
    }
  }

  function toggleThread(threadId: string) {
    selectedThreadIds.value = selectedThreadIds.value.includes(threadId)
      ? selectedThreadIds.value.filter((id) => id !== threadId)
      : [...selectedThreadIds.value, threadId];
  }

  function selectAllVisible() {
    const visible = filteredSessions.value.map((session) => session.threadId);
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
    setDefaultTarget();
    await refreshSessions();
  }

  async function previewSync(): Promise<SyncPreview> {
    if (!activeProfileId.value || !targetProfileId.value || selectedThreadIds.value.length === 0) {
      throw new Error("请选择来源、目标和至少一条会话");
    }
    return backend.previewSync(
      activeProfileId.value,
      targetProfileId.value,
      selectedThreadIds.value,
    );
  }

  async function executeSync(overwriteConflicts: boolean, forceCloseTarget = false) {
    if (!activeProfileId.value || !targetProfileId.value) throw new Error("同步实例不完整");
    syncing.value = true;
    error.value = null;
    try {
      const result = await backend.executeSync(
        activeProfileId.value,
        targetProfileId.value,
        selectedThreadIds.value,
        overwriteConflicts,
        forceCloseTarget,
      );
      lastResult.value = result;
      await refreshSessions();
      return result;
    } catch (reason) {
      error.value = messageOf(reason);
      throw reason;
    } finally {
      syncing.value = false;
    }
  }

  return {
    appState,
    sessions,
    activeProfileId,
    targetProfileId,
    selectedThreadIds,
    search,
    loading,
    syncing,
    error,
    lastResult,
    profiles,
    activeProfile,
    targetProfile,
    filteredSessions,
    selectedSessions,
    initialize,
    refreshSessions,
    selectProfile,
    toggleThread,
    selectAllVisible,
    createProfile,
    deleteProfile,
    previewSync,
    executeSync,
  };
});

function messageOf(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}
