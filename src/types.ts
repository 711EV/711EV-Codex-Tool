export type ProfileKind = "chat_gpt_account" | "custom_api";
export type ProfileMode = "external" | "managed";

export interface Profile {
  id: string;
  name: string;
  kind: ProfileKind;
  mode: ProfileMode;
  codexHome: string;
  providerId: string;
  appPath: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface AppState {
  dataDir: string;
  platform: string;
  profiles: Profile[];
  appServerPath: string | null;
}

export interface SessionLocation {
  profileId: string;
  profileName: string;
  providerId: string;
}

export interface SessionRecord {
  threadId: string;
  title: string;
  cwd: string | null;
  providerId: string | null;
  updatedAt: string | null;
  archived: boolean;
  sizeBytes: number;
  sha256: string;
  locations: SessionLocation[];
}

export type SyncAction =
  | "copy"
  | "update"
  | "skip_identical"
  | "skip_target_ahead"
  | "conflict"
  | "invalid";

export interface SyncPlanItem {
  threadId: string;
  title: string;
  action: SyncAction;
  reason: string;
  sourceSha256: string;
  targetSha256: string | null;
  sizeBytes: number;
}

export interface SyncPreview {
  sourceProfileId: string;
  targetProfileId: string;
  items: SyncPlanItem[];
  copyCount: number;
  updateCount: number;
  skipCount: number;
  conflictCount: number;
  backupBytes: number;
}

export interface SyncResult {
  jobId: string;
  copiedCount: number;
  updatedCount: number;
  skippedCount: number;
  conflictCount: number;
  backupDir: string | null;
  indexRebuilt: boolean;
  warning: string | null;
}

export interface ProfileInput {
  name: string;
  kind: ProfileKind;
  mode: ProfileMode;
  codexHome?: string;
  providerId: string;
  appPath?: string;
}
