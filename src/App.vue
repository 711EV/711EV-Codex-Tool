<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import {
  AlertTriangle,
  Archive,
  ArrowRight,
  Check,
  ChevronRight,
  CircleAlert,
  Database,
  Folder,
  LoaderCircle,
  Plus,
  RefreshCw,
  Search,
  Server,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-vue-next";
import { useWorkspaceStore } from "./stores/workspace";
import type { ProfileInput, SyncAction, SyncPreview } from "./types";

const workspace = useWorkspaceStore();
const profileModalOpen = ref(false);
const preview = ref<SyncPreview | null>(null);
const previewLoading = ref(false);
const overwriteConflicts = ref(false);
const forceClosePrompt = ref(false);
const notice = ref<string | null>(null);
const profileForm = reactive<ProfileInput>({
  name: "",
  kind: "custom_api",
  mode: "managed",
  codexHome: "",
  providerId: "openai",
  appPath: "",
});

const canPreview = computed(
  () =>
    Boolean(workspace.activeProfileId) &&
    Boolean(workspace.targetProfileId) &&
    workspace.selectedThreadIds.length > 0,
);
const allVisibleSelected = computed(
  () =>
    workspace.filteredSessions.length > 0 &&
    workspace.filteredSessions.every((session) =>
      workspace.selectedThreadIds.includes(session.threadId),
    ),
);

onMounted(() => workspace.initialize());

function formatDate(value: string | null) {
  if (!value) return "未知时间";
  return new Intl.DateTimeFormat("zh-CN", {
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

function actionLabel(action: SyncAction) {
  return {
    copy: "新增",
    update: "更新",
    skip_identical: "相同",
    skip_target_ahead: "目标较新",
    conflict: "冲突",
    invalid: "无效",
  }[action];
}

async function openPreview() {
  previewLoading.value = true;
  try {
    preview.value = await workspace.previewSync();
    overwriteConflicts.value = false;
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  } finally {
    previewLoading.value = false;
  }
}

async function runSync(force = false) {
  try {
    const result = await workspace.executeSync(overwriteConflicts.value, force);
    preview.value = null;
    forceClosePrompt.value = false;
    workspace.selectedThreadIds = [];
    notice.value = result.warning ?? `同步完成：新增 ${result.copiedCount} 条，更新 ${result.updatedCount} 条`;
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason);
    if (!force && message.includes("confirm force close and retry")) {
      forceClosePrompt.value = true;
      return;
    }
    notice.value = message;
  }
}

async function submitProfile() {
  try {
    await workspace.createProfile({ ...profileForm });
    profileModalOpen.value = false;
    Object.assign(profileForm, {
      name: "",
      kind: "custom_api",
      mode: "managed",
      codexHome: "",
      providerId: "openai",
      appPath: "",
    });
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  }
}

async function removeProfile(profileId: string, name: string) {
  if (!window.confirm(`移除实例“${name}”？外部 CODEX_HOME 和会话文件不会被删除。`)) return;
  try {
    await workspace.deleteProfile(profileId);
  } catch (reason) {
    notice.value = reason instanceof Error ? reason.message : String(reason);
  }
}
</script>

<template>
  <div class="app-shell">
    <header class="topbar">
      <div class="brand-mark"><Database :size="19" /></div>
      <div class="brand-copy">
        <h1>Codex Local Sync</h1>
        <span>本地会话同步</span>
      </div>
      <div class="topbar-status">
        <span class="status-dot"></span>
        <span>{{ workspace.appState?.platform ?? "正在启动" }}</span>
        <span class="status-divider"></span>
        <span>{{ workspace.appState?.profiles.length ?? 0 }} 个实例</span>
      </div>
    </header>

    <aside class="sidebar">
      <div class="sidebar-heading">
        <span>实例</span>
        <button class="icon-button" title="添加实例" @click="profileModalOpen = true">
          <Plus :size="17" />
        </button>
      </div>
      <nav class="profile-list" aria-label="Codex 实例">
        <button
          v-for="profile in workspace.profiles"
          :key="profile.id"
          class="profile-row"
          :class="{ active: workspace.activeProfileId === profile.id }"
          @click="workspace.selectProfile(profile.id)"
        >
          <span class="profile-icon" :class="profile.kind">
            <Server v-if="profile.kind === 'custom_api'" :size="17" />
            <ShieldCheck v-else :size="17" />
          </span>
          <span class="profile-copy">
            <strong>{{ profile.name }}</strong>
            <small>{{ profile.providerId }}</small>
          </span>
          <ChevronRight :size="15" class="row-chevron" />
        </button>
      </nav>
      <div class="sidebar-footer">
        <Database :size="15" />
        <div>
          <span>便携数据目录</span>
          <strong :title="workspace.appState?.dataDir">{{ workspace.appState?.dataDir ?? "初始化中" }}</strong>
        </div>
      </div>
    </aside>

    <main class="workspace">
      <section class="content-pane">
        <div class="pane-heading">
          <div>
            <p class="eyebrow">来源实例</p>
            <h2>{{ workspace.activeProfile?.name ?? "尚未配置实例" }}</h2>
            <p class="path-line">{{ workspace.activeProfile?.codexHome ?? "添加一个实例以开始扫描" }}</p>
          </div>
          <button
            class="icon-button bordered"
            title="重新扫描"
            :disabled="workspace.loading"
            @click="workspace.refreshSessions"
          >
            <RefreshCw :size="17" :class="{ spinning: workspace.loading }" />
          </button>
        </div>

        <div class="table-tools">
          <label class="search-field">
            <Search :size="16" />
            <input v-model="workspace.search" type="search" placeholder="搜索标题、项目路径或会话 ID" />
          </label>
          <span>{{ workspace.filteredSessions.length }} 条会话</span>
        </div>

        <div v-if="workspace.error" class="inline-error">
          <CircleAlert :size="17" />
          <span>{{ workspace.error }}</span>
        </div>

        <div class="session-table" role="table" aria-label="本地会话">
          <div class="session-head" role="row">
            <label class="check-cell">
              <input
                type="checkbox"
                :checked="allVisibleSelected"
                aria-label="选择所有可见会话"
                @change="workspace.selectAllVisible"
              />
            </label>
            <span>会话</span>
            <span>所在实例</span>
            <span>更新时间</span>
            <span>大小</span>
          </div>

          <div v-if="workspace.loading" class="empty-state">
            <LoaderCircle :size="22" class="spinning" />
            <span>正在扫描本地记录</span>
          </div>
          <div v-else-if="workspace.filteredSessions.length === 0" class="empty-state">
            <Archive :size="23" />
            <strong>没有找到会话</strong>
            <span>检查 CODEX_HOME 路径后重新扫描</span>
          </div>
          <label
            v-for="session in workspace.filteredSessions"
            v-else
            :key="session.threadId"
            class="session-row"
            :class="{ selected: workspace.selectedThreadIds.includes(session.threadId) }"
            role="row"
          >
            <span class="check-cell">
              <input
                type="checkbox"
                :checked="workspace.selectedThreadIds.includes(session.threadId)"
                @change="workspace.toggleThread(session.threadId)"
              />
            </span>
            <span class="session-title-cell">
              <strong>{{ session.title }}</strong>
              <small><Folder :size="13" /> {{ session.cwd ?? session.threadId }}</small>
            </span>
            <span class="location-stack">
              <span v-for="location in session.locations" :key="location.profileId" class="location-pill">
                {{ location.profileName }}
              </span>
            </span>
            <span class="muted-cell">{{ formatDate(session.updatedAt) }}</span>
            <span class="muted-cell">{{ formatBytes(session.sizeBytes) }}</span>
          </label>
        </div>
      </section>

      <aside class="sync-pane">
        <div class="sync-heading">
          <p class="eyebrow">同步设置</p>
          <h2>发送到另一个实例</h2>
        </div>

        <div class="route-block">
          <span class="route-label">来源</span>
          <div class="route-profile">
            <span class="profile-icon compact"><Database :size="16" /></span>
            <div>
              <strong>{{ workspace.activeProfile?.name ?? "未选择" }}</strong>
              <small>{{ workspace.activeProfile?.providerId ?? "-" }}</small>
            </div>
          </div>
          <ArrowRight :size="18" class="route-arrow" />
          <label class="field-label" for="target-profile">目标</label>
          <select id="target-profile" v-model="workspace.targetProfileId">
            <option :value="null" disabled>选择目标实例</option>
            <option
              v-for="profile in workspace.profiles.filter((profile) => profile.id !== workspace.activeProfileId)"
              :key="profile.id"
              :value="profile.id"
            >
              {{ profile.name }} · {{ profile.providerId }}
            </option>
          </select>
        </div>

        <div class="selection-summary">
          <span>已选择</span>
          <strong>{{ workspace.selectedThreadIds.length }}</strong>
          <span>条会话</span>
        </div>
        <div class="summary-meta">
          <span>数据量</span>
          <strong>{{ formatBytes(workspace.selectedSessions.reduce((sum, item) => sum + item.sizeBytes, 0)) }}</strong>
        </div>

        <div class="safety-list">
          <div><Check :size="15" /><span>写入前创建完整备份</span></div>
          <div><Check :size="15" /><span>自动改写目标 Provider</span></div>
          <div><Check :size="15" /><span>冲突默认不覆盖</span></div>
          <div><Check :size="15" /><span>仅处理本地文件</span></div>
        </div>

        <button class="primary-button" :disabled="!canPreview || previewLoading" @click="openPreview">
          <LoaderCircle v-if="previewLoading" :size="17" class="spinning" />
          <ArrowRight v-else :size="17" />
          预览同步
        </button>
      </aside>
    </main>

    <div v-if="profileModalOpen" class="modal-backdrop" @mousedown.self="profileModalOpen = false">
      <form class="modal profile-modal" @submit.prevent="submitProfile">
        <div class="modal-heading">
          <div><p class="eyebrow">Profile</p><h2>添加 Codex 实例</h2></div>
          <button type="button" class="icon-button" title="关闭" @click="profileModalOpen = false"><X :size="18" /></button>
        </div>
        <div class="form-grid">
          <label><span>名称</span><input v-model="profileForm.name" required placeholder="例如：公司 API" /></label>
          <label><span>Provider ID</span><input v-model="profileForm.providerId" required placeholder="openai" /></label>
          <label><span>实例类型</span><select v-model="profileForm.kind"><option value="custom_api">自定义 API</option><option value="chat_gpt_account">ChatGPT 账号</option></select></label>
          <label><span>存储方式</span><select v-model="profileForm.mode"><option value="managed">托管在便携目录</option><option value="external">引用现有 CODEX_HOME</option></select></label>
          <label v-if="profileForm.mode === 'external'" class="full-field"><span>CODEX_HOME</span><input v-model="profileForm.codexHome" required placeholder="完整目录路径" /></label>
          <label class="full-field"><span>客户端路径（可选）</span><input v-model="profileForm.appPath" placeholder="ChatGPT.exe、Codex.exe 或 .app" /></label>
        </div>
        <div class="modal-actions"><button type="button" class="secondary-button" @click="profileModalOpen = false">取消</button><button class="primary-button" type="submit"><Plus :size="17" />添加实例</button></div>
      </form>
    </div>

    <div v-if="preview" class="modal-backdrop" @mousedown.self="preview = null">
      <section class="modal preview-modal">
        <div class="modal-heading">
          <div><p class="eyebrow">执行前检查</p><h2>同步预览</h2></div>
          <button class="icon-button" title="关闭" @click="preview = null"><X :size="18" /></button>
        </div>
        <div class="preview-stats">
          <div><span>新增</span><strong>{{ preview.copyCount }}</strong></div>
          <div><span>更新</span><strong>{{ preview.updateCount }}</strong></div>
          <div><span>跳过</span><strong>{{ preview.skipCount }}</strong></div>
          <div :class="{ danger: preview.conflictCount }"><span>冲突</span><strong>{{ preview.conflictCount }}</strong></div>
        </div>
        <div class="target-disclosure">
          <span>写入目标</span>
          <strong>{{ workspace.targetProfile?.codexHome }}</strong>
          <small v-if="workspace.targetProfile?.mode === 'external'">
            这是外部 CODEX_HOME。继续操作将备份并修改该目录中的本地会话文件。
          </small>
        </div>
        <div class="preview-list">
          <div v-for="item in preview.items" :key="item.threadId" class="preview-row">
            <span class="action-badge" :class="item.action">{{ actionLabel(item.action) }}</span>
            <div><strong>{{ item.title }}</strong><small>{{ item.reason }}</small></div>
            <span>{{ formatBytes(item.sizeBytes) }}</span>
          </div>
        </div>
        <label v-if="preview.conflictCount" class="warning-option">
          <input v-model="overwriteConflicts" type="checkbox" />
          <span><strong>用来源覆盖冲突会话</strong><small>原文件会先保存到备份目录</small></span>
        </label>
        <div class="modal-actions">
          <span class="backup-note"><ShieldCheck :size="16" />预计备份 {{ formatBytes(preview.backupBytes) }}</span>
          <button class="secondary-button" @click="preview = null">取消</button>
          <button class="primary-button" :disabled="workspace.syncing" @click="runSync(false)">
            <LoaderCircle v-if="workspace.syncing" :size="17" class="spinning" />
            <Check v-else :size="17" />
            开始同步
          </button>
        </div>
      </section>
    </div>

    <div v-if="forceClosePrompt" class="modal-backdrop high-priority">
      <section class="modal confirm-modal">
        <div class="danger-icon"><AlertTriangle :size="23" /></div>
        <h2>目标客户端未能正常退出</h2>
        <p>强制结束可能中断正在执行的任务或尚未落盘的数据。仅会处理与目标实例匹配的客户端进程。</p>
        <div class="modal-actions">
          <button class="secondary-button" @click="forceClosePrompt = false">取消同步</button>
          <button class="danger-button" :disabled="workspace.syncing" @click="runSync(true)">
            <Trash2 :size="17" />强制结束并继续
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
