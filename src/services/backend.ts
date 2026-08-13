import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  AppState,
  Profile,
  ProfileInput,
  SessionRecord,
  SyncPreview,
  SyncResult,
} from "../types";

const now = new Date().toISOString();
const demoProfiles: Profile[] = [
  {
    id: "demo-account",
    name: "当前登录账号",
    kind: "chat_gpt_account",
    mode: "external",
    codexHome: "~/.codex",
    providerId: "openai",
    appPath: null,
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

  async previewSync(
    sourceProfileId: string,
    targetProfileId: string,
    threadIds: string[],
  ): Promise<SyncPreview> {
    if (!isTauri()) {
      const source = demoSessions.filter((session) => threadIds.includes(session.threadId));
      return {
        sourceProfileId,
        targetProfileId,
        items: source.map((session) => ({
          threadId: session.threadId,
          title: session.title,
          action: "copy",
          reason: "目标实例中不存在该会话",
          sourceSha256: session.sha256,
          targetSha256: null,
          sizeBytes: session.sizeBytes,
        })),
        copyCount: source.length,
        updateCount: 0,
        skipCount: 0,
        conflictCount: 0,
        backupBytes: 0,
      };
    }
    return invoke<SyncPreview>("preview_sync", {
      sourceProfileId,
      targetProfileId,
      threadIds,
    });
  },

  async executeSync(
    sourceProfileId: string,
    targetProfileId: string,
    threadIds: string[],
    overwriteConflicts: boolean,
    forceCloseTarget = false,
  ): Promise<SyncResult> {
    if (!isTauri()) {
      throw new Error("浏览器预览模式不能执行同步");
    }
    return invoke<SyncResult>("execute_sync", {
      sourceProfileId,
      targetProfileId,
      threadIds,
      overwriteConflicts,
      forceCloseTarget,
    });
  },
};
