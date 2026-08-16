export type ProfileKind = "chat_gpt_account" | "custom_api";

export interface DiscoveredProvider {
  id: string;
  sourceFile: string;
  active: boolean;
}

export interface DiscoveredConfigProfile {
  name: string;
  sourceFile: string;
  providerId: string | null;
  active: boolean;
}

export interface Profile {
  id: string;
  name: string;
  kind: ProfileKind;
  codexHome: string;
  providerId: string;
  appPath: string | null;
  discoverySource: string;
  providers: DiscoveredProvider[];
  configProfiles: DiscoveredConfigProfile[];
  createdAt: string;
  updatedAt: string;
}

export interface AppState {
  dataDir: string;
  platform: string;
  profiles: Profile[];
  appServerPath: string | null;
}

export interface ApplicationUpdate {
  currentVersion: string;
  version: string;
  date: string | null;
  body: string | null;
}

export interface UpdateDownloadProgress {
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
}

export interface DiscoveryReport {
  candidatesScanned: number;
  discoveredCount: number;
  addedCount: number;
  refreshedCount: number;
  profiles: Profile[];
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

export type ThreadSourceKind = "cli" | "vscode" | "internal" | "unknown";
export type ReplicationEligibility =
  | "eligible"
  | "current_provider"
  | "archived"
  | "internal_thread"
  | "invalid_rollout"
  | "already_replicated"
  | "replica"
  | "replica_updated"
  | "source_updated";

export interface ProviderBucket {
  profileId: string;
  providerId: string;
  isCurrent: boolean;
  activeRootThreadCount: number;
  archivedThreadCount: number;
  internalThreadCount: number;
  replicatedCount: number;
  configured?: boolean;
}

export interface ProviderSessionRecord {
  threadId: string;
  providerId: string;
  sourceKind: ThreadSourceKind;
  archived: boolean;
  title: string;
  cwd: string | null;
  updatedAt: string | null;
  sizeBytes: number;
  sha256: string;
  agentNickname: string | null;
  parentThreadId: string | null;
  eligibility: ReplicationEligibility;
  eligibilityReason: string;
  replicaThreadId: string | null;
  isReplica: boolean;
}

export interface ProviderWorkspaceSnapshot {
  providerBuckets: ProviderBucket[];
  selectedProviderId: string | null;
  providerSessions: ProviderSessionRecord[];
}

export interface ArchiveCleanupItem {
  threadId: string;
  title: string;
  providerId: string;
  sourceKind: ThreadSourceKind;
  updatedAt: string | null;
  sizeBytes: number;
}

export interface ArchiveCleanupPreview {
  profileId: string;
  providerId: string;
  items: ArchiveCleanupItem[];
  totalCount: number;
  totalBytes: number;
}

export interface ArchiveCleanupResultItem {
  threadId: string;
  title: string;
  message: string;
}

export interface ArchiveCleanupResult {
  providerId: string;
  deleted: ArchiveCleanupResultItem[];
  failed: ArchiveCleanupResultItem[];
  clientRestarted: boolean;
  warning: string | null;
}

export type InvalidChildCleanupPreview = ArchiveCleanupPreview;
export type InvalidChildCleanupResult = ArchiveCleanupResult;

export type ReplicationAction =
  | "create_replica"
  | "skip_already_replicated"
  | "source_updated"
  | "skip_current_provider"
  | "skip_archived"
  | "skip_internal"
  | "invalid";

export interface ReplicationPlanItem {
  threadId: string;
  title: string;
  sourceProviderId: string;
  action: ReplicationAction;
  reason: string;
  sourceSha256: string;
  replicaThreadId: string | null;
  sizeBytes: number;
}

export interface ReplicationPreview {
  profileId: string;
  targetProviderId: string;
  items: ReplicationPlanItem[];
  createCount: number;
  skipCount: number;
  invalidCount: number;
  estimatedBytes: number;
}

export interface ReplicaResultItem {
  sourceThreadId: string;
  replicaThreadId: string | null;
  title: string;
  status: string;
  message: string;
}

export interface ReplicationResult {
  jobId: string;
  targetProviderId: string;
  created: ReplicaResultItem[];
  skipped: ReplicaResultItem[];
  failed: ReplicaResultItem[];
  clientRestarted: boolean;
  warning: string | null;
}

export type UpdateSyncAction =
  | "source_updated"
  | "replica_updated"
  | "conflict"
  | "invalid";

export interface UpdateSyncPlanItem {
  mappingId: string;
  sourceThreadId: string;
  replicaThreadId: string;
  title: string;
  sourceProviderId: string;
  targetProviderId: string;
  action: UpdateSyncAction;
  reason: string;
}

export interface UpdateSyncPreview {
  profileId: string;
  targetProviderId: string;
  items: UpdateSyncPlanItem[];
  updateCount: number;
  conflictCount: number;
  invalidCount: number;
}

export interface ReplicaMapping {
  id: string;
  profileId: string;
  sourceThreadId: string;
  sourceProviderId: string;
  targetProviderId: string;
  replicaThreadId: string;
  sourceSha256: string;
  replicaSha256: string;
  status: string;
  createdAt: string;
  verifiedAt: string | null;
  deletedAt: string | null;
}

export interface ProviderConfigInput {
  profileId: string;
  providerId: string;
  baseUrl?: string | null;
  apiKey?: string | null;
  template?: string | null;
}

export interface ProviderConfigTemplate {
  id: string;
  fixedProviderId: string;
  fixedBaseUrl: string;
}

export interface ProviderConfigView {
  profileId: string;
  providerId: string;
  baseUrl: string | null;
  envKey: string | null;
  requiresOpenaiAuth: boolean | null;
  experimentalBearerTokenPresent: boolean;
  authJsonApiKeyPresent: boolean;
  activeKeyFilesMatchDatabase: boolean;
  configFile: string;
  authFile: string;
  authKind: string;
  authStorage: string;
  officialAuthSnapshotStatus: string;
  officialAuthCapturedAt: string | null;
  apiKeyMasked: string | null;
  managedByTool: boolean;
  configured: boolean;
  canSwitch: boolean;
  hasPendingChanges: boolean;
  configFingerprint: string;
}

export interface ProviderSwitchResult {
  profileId: string;
  providerId: string;
  configFile: string;
  authFile: string;
  restarted: boolean;
  warning: string | null;
}
