import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  AppState,
  ArchiveCleanupPreview,
  ArchiveCleanupResult,
  DiscoveryReport,
  InvalidChildCleanupPreview,
  InvalidChildCleanupResult,
  Profile,
  ProfileInput,
  ProviderBucket,
  ProviderSessionRecord,
  ProviderWorkspaceSnapshot,
  ReplicaMapping,
  ReplicationPreview,
  ReplicationResult,
  SessionRecord,
  UpdateSyncPreview,
} from "../types";

const now = new Date().toISOString();
const demoProfiles: Profile[] = [
  {
    id: "demo-account",
    name: "当前登录账号",
    kind: "chat_gpt_account",
    mode: "external",
    codexHome: "~/.codex",
    providerId: "custom",
    appPath: null,
    discoverySource: "Codex 默认目录",
    providers: [
      { id: "OpenAI-API", sourceFile: "~/.codex/config.toml", active: false },
      { id: "SHUAI-API", sourceFile: "~/.codex/config.toml", active: false },
      { id: "custom", sourceFile: "~/.codex/config.toml", active: true },
    ],
    configProfiles: [],
    createdAt: now,
    updatedAt: now,
  },
  {
    id: "demo-relay",
    name: "开发 API",
    kind: "custom_api",
    mode: "managed",
    codexHome: "CodexLocalSync.data/profiles/development-api",
    providerId: "relay",
    appPath: null,
    discoverySource: "Codex Local Sync 托管实例",
    providers: [
      { id: "openai", sourceFile: "CodexLocalSync.data/profiles/development-api/config.toml", active: false },
      { id: "relay", sourceFile: "CodexLocalSync.data/profiles/development-api/config.toml", active: true },
    ],
    configProfiles: [
      {
        name: "development",
        sourceFile: "CodexLocalSync.data/profiles/development-api/development.config.toml",
        providerId: "relay",
        active: true,
      },
    ],
    createdAt: now,
    updatedAt: now,
  },
];

const demoSessions: SessionRecord[] = [
  {
    threadId: "019b3e6d-demo-1",
    title: "实现登录态切换与会话恢复",
    cwd: "F:\\Projects\\desktop-app",
    providerId: "openai",
    updatedAt: new Date(Date.now() - 12 * 60_000).toISOString(),
    archived: false,
    sizeBytes: 294_302,
    sha256: "demo-a",
    locations: [
      { profileId: "demo-account", profileName: "当前登录账号", providerId: "openai" },
    ],
  },
  {
    threadId: "019b3e6d-demo-2",
    title: "检查同步冲突策略",
    cwd: "/Users/demo/work/codex-sync",
    providerId: "relay",
    updatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
    archived: false,
    sizeBytes: 118_920,
    sha256: "demo-b",
    locations: [
      { profileId: "demo-relay", profileName: "开发 API", providerId: "relay" },
    ],
  },
];

const demoProviderBuckets: ProviderBucket[] = [
  {
    profileId: "demo-account",
    providerId: "openai",
    isCurrent: false,
    activeRootThreadCount: 0,
    archivedThreadCount: 0,
    internalThreadCount: 0,
    replicatedCount: 0,
  },
  {
    profileId: "demo-account",
    providerId: "custom",
    isCurrent: true,
    activeRootThreadCount: 3,
    archivedThreadCount: 0,
    internalThreadCount: 1,
    replicatedCount: 0,
  },
  {
    profileId: "demo-account",
    providerId: "OpenAI-API",
    isCurrent: false,
    activeRootThreadCount: 9,
    archivedThreadCount: 1,
    internalThreadCount: 1,
    replicatedCount: 2,
  },
  {
    profileId: "demo-account",
    providerId: "SHUAI-API",
    isCurrent: false,
    activeRootThreadCount: 6,
    archivedThreadCount: 0,
    internalThreadCount: 1,
    replicatedCount: 0,
  },
];

const demoProviderSessions: ProviderSessionRecord[] = [
  {
    threadId: "019b3e6d-demo-1",
    providerId: "OpenAI-API",
    sourceKind: "vscode",
    archived: false,
    title: "实现登录态切换与会话恢复",
    cwd: "F:\\Projects\\desktop-app",
    updatedAt: new Date(Date.now() - 12 * 60_000).toISOString(),
    sizeBytes: 294_302,
    sha256: "demo-a",
    agentNickname: null,
    parentThreadId: null,
    eligibility: "eligible",
    eligibilityReason: "可复制到当前 Provider",
    replicaThreadId: null,
    isReplica: false,
  },
  {
    threadId: "019b3e6d-demo-2",
    providerId: "OpenAI-API",
    sourceKind: "cli",
    archived: false,
    title: "检查会话索引修复",
    cwd: "F:\\Projects\\codex-sync",
    updatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
    sizeBytes: 118_920,
    sha256: "demo-b",
    agentNickname: null,
    parentThreadId: null,
    eligibility: "already_replicated",
    eligibilityReason: "已存在经过验证的独立副本",
    replicaThreadId: "019b3e6d-copy-2",
    isReplica: false,
  },
  {
    threadId: "019b3e6d-demo-child",
    providerId: "OpenAI-API",
    sourceKind: "internal",
    archived: false,
    title: "检查登录态切换结果",
    cwd: "F:\\Projects\\desktop-app",
    updatedAt: new Date(Date.now() - 8 * 60_000).toISOString(),
    sizeBytes: 34_816,
    sha256: "demo-child",
    agentNickname: "Boyle",
    parentThreadId: "019b3e6d-demo-1",
    eligibility: "internal_thread",
    eligibilityReason: "仅支持 cli/vscode 主会话",
    replicaThreadId: null,
    isReplica: false,
  },
];

export const backend = {
  async getAppState(): Promise<AppState> {
    if (!isTauri()) {
      return {
        dataDir: "CodexLocalSync.data",
        platform: "browser-preview",
        profiles: demoProfiles,
        appServerPath: null,
      };
    }
    return invoke<AppState>("get_app_state");
  },

  async createProfile(input: ProfileInput): Promise<Profile> {
    if (!isTauri()) {
      throw new Error("浏览器预览模式不能修改 Profile");
    }
    return invoke<Profile>("create_profile", { input });
  },

  async deleteProfile(profileId: string): Promise<void> {
    if (!isTauri()) {
      throw new Error("浏览器预览模式不能修改 Profile");
    }
    return invoke("delete_profile", { profileId });
  },

  async discoverProfiles(): Promise<DiscoveryReport> {
    if (!isTauri()) {
      return {
        candidatesScanned: 6,
        discoveredCount: demoProfiles.length,
        addedCount: 0,
        refreshedCount: demoProfiles.length,
        profiles: demoProfiles,
      };
    }
    return invoke<DiscoveryReport>("discover_profiles");
  },

  async scanSessions(profileId?: string): Promise<SessionRecord[]> {
    if (!isTauri()) {
      return profileId
        ? demoSessions.filter((session) =>
            session.locations.some((location) => location.profileId === profileId),
          )
        : demoSessions;
    }
    return invoke<SessionRecord[]>("scan_sessions", { profileId: profileId ?? null });
  },

  async providerWorkspace(
    profileId: string,
    providerId: string | null,
  ): Promise<ProviderWorkspaceSnapshot> {
    if (!isTauri()) {
      const providerBuckets = demoProviderBuckets.map((bucket) => ({ ...bucket, profileId }));
      const selectedProviderId = providerBuckets.some((bucket) => bucket.providerId === providerId)
        ? providerId
        : providerBuckets.find((bucket) => !bucket.isCurrent && bucket.activeRootThreadCount > 0)
          ?.providerId ?? providerBuckets[0]?.providerId ?? null;
      const providerSessions = selectedProviderId === "OpenAI-API"
        ? demoProviderSessions
        : demoProviderSessions
        .filter(() => selectedProviderId === "custom")
        .map((session, index) => ({
          ...session,
          threadId: `current-${index}`,
          providerId: "custom",
          eligibility: "current_provider" as const,
          eligibilityReason: "会话已经属于当前 Provider",
        }));
      return { providerBuckets, selectedProviderId, providerSessions };
    }
    return invoke<ProviderWorkspaceSnapshot>("provider_workspace", {
      profileId,
      providerId,
    });
  },

  async archiveCleanupPreview(
    profileId: string,
    providerId: string,
  ): Promise<ArchiveCleanupPreview> {
    if (!isTauri()) {
      const items = demoProviderSessions
        .filter((session) => session.providerId === providerId && session.archived)
        .map((session) => ({
          threadId: session.threadId,
          title: session.title,
          providerId: session.providerId,
          sourceKind: session.sourceKind,
          updatedAt: session.updatedAt,
          sizeBytes: session.sizeBytes,
        }));
      return {
        profileId,
        providerId,
        totalCount: items.length,
        totalBytes: items.reduce((sum, item) => sum + item.sizeBytes, 0),
        items,
      };
    }
    return invoke<ArchiveCleanupPreview>("archive_cleanup_preview", {
      profileId,
      providerId,
    });
  },

  async archiveCleanupExecute(
    profileId: string,
    providerId: string,
    threadIds: string[],
    forceCloseClient = false,
  ): Promise<ArchiveCleanupResult> {
    if (!isTauri()) {
      return {
        providerId,
        deleted: threadIds.map((threadId) => ({
          threadId,
          title: threadId,
          message: "已永久删除归档会话",
        })),
        failed: [],
        clientRestarted: false,
        warning: null,
      };
    }
    return invoke<ArchiveCleanupResult>("archive_cleanup_execute", {
      profileId,
      providerId,
      threadIds,
      forceCloseClient,
    });
  },

  async invalidChildCleanupPreview(
    profileId: string,
    providerId: string,
  ): Promise<InvalidChildCleanupPreview> {
    if (!isTauri()) {
      const items = demoProviderSessions
        .filter((session) =>
          session.providerId === providerId &&
          session.sourceKind === "internal" &&
          !session.archived
        )
        .map((session) => ({
          threadId: session.threadId,
          title: session.title,
          providerId: session.providerId,
          sourceKind: session.sourceKind,
          updatedAt: session.updatedAt,
          sizeBytes: session.sizeBytes,
        }));
      return {
        profileId,
        providerId,
        totalCount: items.length,
        totalBytes: items.reduce((sum, item) => sum + item.sizeBytes, 0),
        items,
      };
    }
    return invoke<InvalidChildCleanupPreview>("invalid_child_cleanup_preview", {
      profileId,
      providerId,
    });
  },

  async invalidChildCleanupExecute(
    profileId: string,
    providerId: string,
    threadIds: string[],
    forceCloseClient = false,
  ): Promise<InvalidChildCleanupResult> {
    if (!isTauri()) {
      return {
        providerId,
        deleted: threadIds.map((threadId) => ({
          threadId,
          title: threadId,
          message: "已永久删除子会话",
        })),
        failed: [],
        clientRestarted: false,
        warning: null,
      };
    }
    return invoke<InvalidChildCleanupResult>("invalid_child_cleanup_execute", {
      profileId,
      providerId,
      threadIds,
      forceCloseClient,
    });
  },

  async replicationPreview(
    profileId: string,
    sourceThreadIds: string[],
  ): Promise<ReplicationPreview> {
    if (!isTauri()) {
      const selected = demoProviderSessions.filter((session) =>
        sourceThreadIds.includes(session.threadId),
      );
      const items = selected.map((session) => ({
        threadId: session.threadId,
        title: session.title,
        sourceProviderId: session.providerId,
        action: session.eligibility === "eligible" ? "create_replica" as const : "skip_already_replicated" as const,
        reason: session.eligibilityReason,
        sourceSha256: session.sha256,
        replicaThreadId: session.replicaThreadId,
        sizeBytes: session.sizeBytes,
      }));
      return {
        profileId,
        targetProviderId: "custom",
        items,
        createCount: items.filter((item) => item.action === "create_replica").length,
        skipCount: items.filter((item) => item.action === "skip_already_replicated").length,
        invalidCount: 0,
        estimatedBytes: items
          .filter((item) => item.action === "create_replica")
          .reduce((sum, item) => sum + item.sizeBytes, 0),
      };
    }
    return invoke<ReplicationPreview>("replication_preview", {
      profileId,
      sourceThreadIds,
    });
  },

  async replicationExecute(
    profileId: string,
    sourceThreadIds: string[],
    forceCloseClient = false,
  ): Promise<ReplicationResult> {
    if (!isTauri()) {
      const preview = await this.replicationPreview(profileId, sourceThreadIds);
      return {
        jobId: "demo-job",
        targetProviderId: preview.targetProviderId,
        created: preview.items
          .filter((item) => item.action === "create_replica")
          .map((item) => ({
            sourceThreadId: item.threadId,
            replicaThreadId: `${item.threadId}-copy`,
            title: item.title,
            status: "verified",
            message: "已创建新会话并通过当前 Provider 验证",
          })),
        skipped: [],
        failed: [],
        clientRestarted: false,
        warning: null,
      };
    }
    return invoke<ReplicationResult>("replication_execute", {
      profileId,
      sourceThreadIds,
      forceCloseClient,
    });
  },

  async replicationMigrate(
    profileId: string,
    sourceThreadIds: string[],
    forceCloseClient = false,
  ): Promise<ReplicationResult> {
    if (!isTauri()) {
      const preview = await this.replicationPreview(profileId, sourceThreadIds);
      return {
        jobId: "demo-migration-job",
        targetProviderId: preview.targetProviderId,
        created: preview.items
          .filter((item) => item.action === "create_replica")
          .map((item) => ({
            sourceThreadId: item.threadId,
            replicaThreadId: `${item.threadId}-migrated`,
            title: item.title,
            status: "migrated",
            message: "已迁移到当前供应商并删除来源会话",
          })),
        skipped: [],
        failed: [],
        clientRestarted: false,
        warning: null,
      };
    }
    return invoke<ReplicationResult>("replication_migrate", {
      profileId,
      sourceThreadIds,
      forceCloseClient,
    });
  },

  async restartCodexClient(
    profileId: string,
    forceCloseClient = false,
  ): Promise<boolean> {
    if (!isTauri()) return true;
    return invoke<boolean>("restart_codex_client", {
      profileId,
      forceCloseClient,
    });
  },

  async replicationSyncUpdates(
    profileId: string,
    forceCloseClient = false,
  ): Promise<ReplicationResult> {
    if (!isTauri()) {
      return {
        jobId: "demo-update-sync-job",
        targetProviderId: "custom",
        created: [],
        skipped: [],
        failed: [],
        clientRestarted: false,
        warning: null,
      };
    }
    return invoke<ReplicationResult>("replication_sync_updates", {
      profileId,
      forceCloseClient,
    });
  },

  async replicationSyncPreview(profileId: string): Promise<UpdateSyncPreview> {
    if (!isTauri()) {
      return {
        profileId,
        targetProviderId: "custom",
        items: [],
        updateCount: 0,
        conflictCount: 0,
        invalidCount: 0,
      };
    }
    return invoke<UpdateSyncPreview>("replication_sync_preview", { profileId });
  },

  async replicationHistory(profileId: string): Promise<ReplicaMapping[]> {
    if (!isTauri()) return [];
    return invoke<ReplicaMapping[]>("replication_history", { profileId });
  },

};
