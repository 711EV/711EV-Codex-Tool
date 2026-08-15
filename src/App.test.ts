// @vitest-environment jsdom

import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App.vue";
import { backend } from "./services/backend";
import { useWorkspaceStore } from "./stores/workspace";

describe("Codex Provider Sync workspace", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("groups sessions by Provider and fixes the current Provider as target", async () => {
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    expect(wrapper.find(".desktop-titlebar").exists()).toBe(true);
    expect(wrapper.find(".desktop-titlebar-name").text()).toBe("711EV-Codex-Tool");
    expect(wrapper.find(".desktop-titlebar-logo").attributes("src")).toContain("titlebar-logo-clear.png");
    expect(wrapper.findAll(".desktop-window-button")).toHaveLength(3);
    expect(wrapper.find(".sidebar-heading > span").text()).toBe("供应商");
    expect(wrapper.findAll(".sidebar-actions button")).toHaveLength(1);
    expect(wrapper.get('[data-testid="rediscover-providers"]').attributes("title"))
      .toBe("重新发现供应商");
    expect(wrapper.find('.sidebar-actions button[title="添加 CODEX_HOME"]').exists()).toBe(false);
    expect(wrapper.find(".profile-modal").exists()).toBe(false);
    expect(wrapper.find(".sidebar-version-brand").text()).toBe("by 711EV");
    expect(wrapper.find(".sidebar-version-number").text()).toBe(`版本号 ${__APP_VERSION__}`);
    expect(wrapper.find(".top-menu-storage").attributes("title")).toBe("~/.codex");
    expect(wrapper.find(".top-menu-storage").attributes("data-tooltip")).toBeUndefined();
    expect(wrapper.get('[data-testid="storage-location-switch"]').attributes("title"))
      .toBe("切换存储位置");
    expect((wrapper.get('[data-testid="storage-location-switch"]').element as HTMLButtonElement).disabled)
      .toBe(false);
    expect(wrapper.find(".home-picker").exists()).toBe(false);
    expect(wrapper.findAll(".top-menu-link").map((item) => item.text())).toEqual([
      "交流群",
      "711EV导航",
      "711EV中转站",
    ]);
    expect(wrapper.text()).toContain("OpenAI-API");
    expect(wrapper.text()).toContain("custom");
    expect(wrapper.find(".provider-list").text()).not.toContain("条主会话");
    expect(wrapper.findAll(".provider-kind-tag").map((tag) => tag.text())).toContain("官方");
    expect(wrapper.findAll(".provider-kind-tag").map((tag) => tag.text())).toContain("中转");
    expect(wrapper.findAll(".provider-row .provider-openai-logo")).toHaveLength(1);
    expect(wrapper.findAll(".provider-row .provider-relay-icon")).toHaveLength(3);
    expect(wrapper.findAll(".current-mark")).toHaveLength(0);
    expect(wrapper.find(".provider-current-usage").exists()).toBe(false);
    expect(wrapper.find(".provider-list").text()).not.toContain("使用中");
    expect(wrapper.findAll(".provider-status-dot.success")).toHaveLength(1);
    expect(wrapper.findAll(".provider-status-dot.info")).toHaveLength(3);
    expect(wrapper.findAll(".provider-row")[0].text()).toContain("openai");
    expect(wrapper.findAll(".provider-row")[0].text()).toContain("官方");
    expect(wrapper.find(".current-provider").classes()).not.toContain("active");
    expect(wrapper.findAll(".row-chevron")).toHaveLength(0);
    expect(wrapper.find(".provider-row .profile-copy strong").attributes("title")).toBeTruthy();
    expect(wrapper.find(".pane-heading .eyebrow").exists()).toBe(false);
    expect(wrapper.find(".pane-heading .path-line").exists()).toBe(false);
    expect(wrapper.find(".pane-heading .table-session-counts").exists()).toBe(false);
    expect(wrapper.get('[data-testid="refresh-sessions"]').text()).toBe("刷新会话");
    expect(wrapper.get('[data-testid="cleanup-child-sessions"]').text()).toBe("清理子会话");
    expect(wrapper.get('[data-testid="cleanup-archived-sessions"]').text()).toBe("清理归档");
    expect(wrapper.find('[data-testid="sync-updated-sessions"]').exists()).toBe(false);
    expect(wrapper.find(".table-tools .table-session-counts").text()).toContain("总会话 3 条");
    expect(wrapper.text()).toContain("实现登录态切换与会话恢复");
    expect(wrapper.find(".sync-heading").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("复制到custom");
    expect(wrapper.get(".sync-provider-config-section").text()).toContain("供应商配置");
    expect(wrapper.get(".sync-provider-config-section").text()).toContain("待开发");
    expect(wrapper.findAll(".sync-pane > section")).toHaveLength(2);
    expect(wrapper.findAll(".provider-route-card")).toHaveLength(2);
    expect(wrapper.findAll(".provider-route-node .route-label").map((label) => label.text()))
      .toEqual(["来源供应商", "目标供应商"]);
    expect(wrapper.get(".session-route-summary-card").text()).toContain("会话来源OpenAI-API");
    expect(wrapper.get(".session-route-summary-card").text()).toContain("会话目标custom");
    expect(wrapper.get(".session-route-summary-card").text()).toContain("预计新增0 B");
    const replicationContent = wrapper.get(".sync-replication-section .replication-content");
    expect(wrapper.find(".sync-replication-section .replication-card").exists()).toBe(false);
    expect(replicationContent.find(".provider-route").exists()).toBe(true);
    const summaryCard = replicationContent.get(".replication-summary-card");
    expect(summaryCard.find(".session-route-summary-card").exists()).toBe(true);
    expect(summaryCard.get(".selection-summary").text()).toBe("已选择0条会话");
    expect(wrapper.find(".sync-pane-footer .selection-summary").exists()).toBe(false);
    expect(wrapper.findAll(".sync-pane-actions button").map((button) => button.text()))
      .toEqual(["复制", "迁移"]);
    expect(wrapper.findAll(".sync-pane-actions button")[0].attributes("title"))
      .toBe("复制到当前供应商并保留来源会话");
    expect(wrapper.findAll(".sync-pane-actions button")[1].attributes("title"))
      .toBe("迁移到当前供应商并删除来源会话");
    expect(wrapper.get(".sync-pane-footer").findAll("button")).toHaveLength(2);
    expect(wrapper.text()).not.toContain("复制目标");
    expect(wrapper.text()).not.toContain("目标 Provider");
    expect(wrapper.text()).not.toContain("发送到另一个实例");
    expect(wrapper.text()).not.toContain("写入前创建完整备份");
    expect(wrapper.findAll(".session-row").every((row) => row.attributes("title") === undefined)).toBe(true);
  });

  it("switches storage locations from the top menu and stays visible with one location", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const switchButton = wrapper.get('[data-testid="storage-location-switch"]');
    await switchButton.trigger("click");

    const options = wrapper.findAll(".storage-location-option");
    expect(options).toHaveLength(2);
    expect(options.map((option) => option.text())).toEqual([
      expect.stringContaining("当前登录账号"),
      expect.stringContaining("开发 API"),
    ]);

    await options[1].trigger("click");
    await flushPromises();

    expect(workspace.activeProfileId).toBe("demo-relay");
    expect(wrapper.get(".top-menu-storage").attributes("title"))
      .toBe("CodexLocalSync.data/profiles/development-api");
    expect(wrapper.find(".storage-location-menu").exists()).toBe(false);

    const activeProfile = workspace.activeProfile!;
    workspace.appState!.profiles = [activeProfile];
    await nextTick();

    const singleLocationButton = wrapper.get('[data-testid="storage-location-switch"]');
    expect((singleLocationButton.element as HTMLButtonElement).disabled).toBe(true);
    expect(singleLocationButton.attributes("title")).toBe("暂无其他存储位置");
  });

  it("rediscovers Providers only in the active storage location", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const snapshot = {
      providerBuckets: workspace.providerBuckets.map((bucket) => ({ ...bucket })),
      selectedProviderId: workspace.selectedProviderId,
      providerSessions: workspace.providerSessions.map((session) => ({ ...session })),
    };
    const providerWorkspace = vi.spyOn(backend, "providerWorkspace").mockResolvedValue(snapshot);
    const discoverProfiles = vi.spyOn(backend, "discoverProfiles");

    await wrapper.get('[data-testid="rediscover-providers"]').trigger("click");
    await flushPromises();

    expect(providerWorkspace).toHaveBeenCalledTimes(1);
    expect(providerWorkspace).toHaveBeenCalledWith(
      workspace.activeProfileId,
      workspace.selectedProviderId,
    );
    expect(discoverProfiles).not.toHaveBeenCalled();
    expect(wrapper.get(".toast").text()).toContain(
      `已重新发现当前存储位置下的 ${workspace.providerBuckets.length} 个供应商`,
    );
  });

  it("uses the same official and relay icons for the vertical copy route", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    workspace.selectedProviderId = "openai";
    await nextTick();

    const cards = wrapper.findAll(".provider-route-card");
    expect(cards[0].get("strong").text()).toBe("openai");
    expect(cards[0].find(".provider-openai-logo").exists()).toBe(true);
    expect(cards[0].find(".provider-relay-icon").exists()).toBe(false);
    expect(cards[1].get("strong").text()).toBe(workspace.currentProviderId);
    expect(cards[1].find(".provider-relay-icon").exists()).toBe(true);
    expect(cards[1].find(".provider-openai-logo").exists()).toBe(false);
  });

  it("marks the current Provider beside the middle heading", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    expect(wrapper.find(".current-provider-indicator").exists()).toBe(false);

    workspace.selectedProviderId = workspace.currentProviderId;
    await nextTick();

    const heading = wrapper.get(".pane-heading-title");
    expect(heading.get(".pane-provider-cluster").classes()).toContain("pane-provider-cluster");
    expect(heading.get("h2").text()).toBe(workspace.currentProviderId);
    expect(heading.get(".pane-provider-name").attributes("title"))
      .toBe(workspace.currentProviderId);
    expect(heading.get(".pane-provider-name").classes()).toContain("pane-provider-name");
    expect(heading.get(".current-provider-indicator").text()).toBe("正在使用中");
    expect(heading.find(".current-provider-indicator .provider-status-dot.success").exists())
      .toBe(true);
  });

  it("nests linked child sessions and leaves unlinked children at the root", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    expect(wrapper.findAll(".session-expand-button")).toHaveLength(1);
    expect(wrapper.findAll(".child-session-row")).toHaveLength(0);
    expect(wrapper.text()).not.toContain("检查登录态切换结果");
    const rootTitle = wrapper.get(".session-row .session-title-copy strong");
    expect(rootTitle.attributes("title")).toBe(rootTitle.text());

    await wrapper.get(".session-expand-button").trigger("click");
    expect(wrapper.get(".session-expand-button").attributes("aria-expanded")).toBe("true");
    expect(wrapper.get(".child-session-row").text()).toContain("检查登录态切换结果");
    expect(wrapper.get(".child-session-row .muted-cell").text()).toBe("Boyle");
    const childTitle = wrapper.get(".child-session-row .session-title-copy strong");
    expect(childTitle.attributes("title")).toBe(childTitle.text());

    const workspace = useWorkspaceStore(pinia);
    workspace.providerSessions.push({
      ...workspace.providerSessions.find((session) => session.sourceKind === "internal")!,
      threadId: "unlinked-child",
      title: "未关联子会话",
      agentNickname: null,
      parentThreadId: null,
    });
    await nextTick();

    const unlinkedRow = wrapper.findAll(".session-row").find((row) => row.text().includes("未关联子会话"));
    expect(unlinkedRow?.exists()).toBe(true);
    expect(unlinkedRow?.classes()).not.toContain("child-session-row");
    expect(unlinkedRow?.find(".muted-cell").text()).toBe("未命名");
  });

  it("opens top menu links in the system browser fallback", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    await wrapper.findAll(".top-menu-link")[0].trigger("click");
    expect(open).toHaveBeenCalledWith(
      "https://qm.qq.com/q/e9xHZxgN4Q",
      "_blank",
      "noopener,noreferrer",
    );
    open.mockRestore();
  });

  it("shows the scanning overlay during initial startup", async () => {
    const appState = await backend.getAppState();
    let finishInitialization!: () => void;
    vi.spyOn(backend, "getAppState").mockImplementationOnce(
      () => new Promise((resolve) => {
        finishInitialization = () => resolve(appState);
      }),
    );
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });

    await flushPromises();
    expect(wrapper.find(".scan-overlay").exists()).toBe(true);
    expect(wrapper.find(".scan-overlay-content").text()).toContain("正在加载供应商和本地会话");

    finishInitialization();
    await flushPromises();
    expect(wrapper.find(".scan-overlay").exists()).toBe(false);
  });

  it("shows a scanning overlay while switching Provider", async () => {
    vi.useFakeTimers();
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    let finishScan!: () => void;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });
    vi.spyOn(backend, "providerWorkspace").mockImplementationOnce(
      () => new Promise((resolve) => {
        finishScan = () => resolve({
          providerBuckets: [],
          selectedProviderId: null,
          providerSessions: [],
        });
      }),
    );
    const target = wrapper.findAll(".provider-row").find((row) => !row.classes("active"));

    await target?.trigger("click");
    await flushPromises();
    expect(wrapper.find(".scan-overlay").exists()).toBe(true);
    expect(wrapper.find(".scan-overlay-content").text()).toContain("正在扫描");
    expect(wrapper.find(".scan-progress").exists()).toBe(true);
    expect(wrapper.find(".session-table .spinning").exists()).toBe(false);

    finishScan();
    await flushPromises();
    await vi.advanceTimersByTimeAsync(500);
    await flushPromises();
    expect(wrapper.find(".scan-overlay").exists()).toBe(false);
  });

  it("shows the inline loading state only for a manual content refresh", async () => {
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    let finishRefresh!: () => void;
    vi.spyOn(backend, "providerWorkspace").mockImplementationOnce(
      () => new Promise((resolve) => {
        finishRefresh = () => resolve({
          providerBuckets: [],
          selectedProviderId: null,
          providerSessions: [],
        });
      }),
    );

    await wrapper.get('[data-testid="refresh-sessions"]').trigger("click");
    await flushPromises();
    expect(wrapper.find(".scan-overlay").exists()).toBe(false);
    expect(wrapper.find(".session-table .empty-state").text()).toContain("正在扫描本地 Codex 会话");

    finishRefresh();
    await flushPromises();
    expect(wrapper.find(".session-table .spinning").exists()).toBe(false);
  });

  it("shows update sync only for the current Provider and keeps the action responsive", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    workspace.selectedProviderId = workspace.currentProviderId;
    await nextTick();

    const syncPreview = vi.spyOn(backend, "replicationSyncPreview").mockResolvedValue({
      profileId: workspace.activeProfileId!,
      targetProviderId: workspace.currentProviderId!,
      items: [{
        mappingId: "mapping",
        sourceThreadId: "source",
        replicaThreadId: "replica",
        title: "待同步会话",
        sourceProviderId: "openai",
        targetProviderId: workspace.currentProviderId!,
        action: "source_updated",
        reason: "将用来源的最新内容更新现有副本",
      }],
      updateCount: 1,
      conflictCount: 0,
      invalidCount: 0,
    });
    const syncUpdates = vi.spyOn(backend, "replicationSyncUpdates").mockResolvedValue({
      jobId: "sync-updates",
      targetProviderId: workspace.currentProviderId!,
      created: [{
        sourceThreadId: "source",
        replicaThreadId: "replica",
        title: "待同步会话",
        status: "synchronized",
        message: "已用来源的最新内容更新现有副本",
      }],
      skipped: [],
      failed: [],
      clientRestarted: false,
      warning: null,
    });
    const button = wrapper.get('[data-testid="sync-updated-sessions"]');
    expect(button.text()).toBe("同步会话");
    expect(wrapper.findAll(".pane-heading-actions .pane-action-button").map((item) => item.text()))
      .toEqual(["同步会话", "清理子会话", "清理归档", "刷新会话"]);

    await button.trigger("click");
    await flushPromises();

    expect(syncPreview).toHaveBeenCalledWith(workspace.activeProfileId);
    expect(syncUpdates).not.toHaveBeenCalled();
    expect(wrapper.get(".update-sync-modal").text()).toContain("需要同步1");
    expect(wrapper.get(".update-sync-modal").text()).toContain("待同步会话");

    await wrapper.get(".update-sync-modal .primary-button").trigger("click");
    await flushPromises();

    expect(syncUpdates).toHaveBeenCalledWith(workspace.activeProfileId, false);
    expect(wrapper.get(".update-sync-modal").text()).toContain("同步结果");
    expect(wrapper.get(".update-sync-modal").text()).toContain("已更新1");
  });

  it("previews archived sessions before cleaning the selected Provider", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const providerId = workspace.selectedProviderId!;
    const cleanupPreview = vi.spyOn(backend, "archiveCleanupPreview").mockResolvedValue({
      profileId: workspace.activeProfileId!,
      providerId,
      items: [{
        threadId: "archived-thread",
        title: "待清理归档会话",
        providerId,
        sourceKind: "vscode",
        updatedAt: "2026-08-15T10:00:00Z",
        sizeBytes: 2048,
      }],
      totalCount: 1,
      totalBytes: 2048,
    });
    const cleanupExecute = vi.spyOn(backend, "archiveCleanupExecute").mockResolvedValue({
      providerId,
      deleted: [{
        threadId: "archived-thread",
        title: "待清理归档会话",
        message: "已永久删除归档会话",
      }],
      failed: [],
      clientRestarted: false,
      warning: null,
    });

    await wrapper.get('[data-testid="cleanup-archived-sessions"]').trigger("click");
    await flushPromises();

    expect(cleanupPreview).toHaveBeenCalledWith(workspace.activeProfileId, providerId);
    expect(cleanupExecute).not.toHaveBeenCalled();
    expect(wrapper.get(".archive-cleanup-modal").text()).toContain("归档会话1");
    expect(wrapper.get(".archive-cleanup-modal").text()).toContain("待清理归档会话");
    expect(wrapper.get(".archive-cleanup-modal").text()).toContain("永久删除且不会备份");

    await wrapper.get(".archive-cleanup-modal .danger-button").trigger("click");
    await flushPromises();

    expect(cleanupExecute).toHaveBeenCalledWith(
      workspace.activeProfileId,
      providerId,
      ["archived-thread"],
      false,
    );
    expect(wrapper.get(".archive-cleanup-modal").text()).toContain("清理结果");
    expect(wrapper.get(".archive-cleanup-modal").text()).toContain("已删除1");
  });

  it("previews and reports invalid child cleanup for the selected Provider", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const providerId = workspace.selectedProviderId!;
    const cleanupPreview = vi.spyOn(backend, "invalidChildCleanupPreview").mockResolvedValue({
      profileId: workspace.activeProfileId!,
      providerId,
      items: [{
        threadId: "child-thread-1",
        title: "待清理子会话",
        providerId,
        sourceKind: "internal",
        updatedAt: "2026-08-15T10:00:00Z",
        sizeBytes: 3072,
      }, {
        threadId: "child-thread-2",
        title: "无法清理子会话",
        providerId,
        sourceKind: "internal",
        updatedAt: "2026-08-15T11:00:00Z",
        sizeBytes: 1024,
      }],
      totalCount: 2,
      totalBytes: 4096,
    });
    const cleanupExecute = vi.spyOn(backend, "invalidChildCleanupExecute").mockResolvedValue({
      providerId,
      deleted: [{
        threadId: "child-thread-1",
        title: "待清理子会话",
        message: "已永久删除子会话",
      }],
      failed: [{
        threadId: "child-thread-2",
        title: "无法清理子会话",
        message: "文件仍被占用",
      }],
      clientRestarted: false,
      warning: null,
    });

    const button = wrapper.get('[data-testid="cleanup-child-sessions"]');
    expect(button.text()).toBe("清理子会话");
    await button.trigger("click");
    await flushPromises();

    expect(cleanupPreview).toHaveBeenCalledWith(workspace.activeProfileId, providerId);
    expect(cleanupExecute).not.toHaveBeenCalled();
    expect(wrapper.get(".child-cleanup-modal").text()).toContain("未归档子会话2");
    expect(wrapper.get(".child-cleanup-modal").text()).toContain("4.0 KB");
    expect(wrapper.get(".child-cleanup-modal").text()).toContain("永久删除且不会备份");
    expect(wrapper.get(".child-cleanup-impact-note").text()).toContain(
      "删除子会话不会影响其所属主会话",
    );

    await wrapper.get(".child-cleanup-modal .danger-button").trigger("click");
    await flushPromises();

    expect(cleanupExecute).toHaveBeenCalledWith(
      workspace.activeProfileId,
      providerId,
      ["child-thread-1", "child-thread-2"],
      false,
    );
    const resultText = wrapper.get(".child-cleanup-modal").text();
    expect(resultText).toContain("已删除1");
    expect(resultText).toContain("失败1");
    expect(resultText).toContain("已永久删除子会话");
    expect(resultText).toContain("文件仍被占用");
  });

  it("retries child cleanup with force close after client shutdown fails", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const providerId = workspace.selectedProviderId!;
    vi.spyOn(backend, "invalidChildCleanupPreview").mockResolvedValue({
      profileId: workspace.activeProfileId!,
      providerId,
      items: [{
        threadId: "occupied-child",
        title: "客户端占用的子会话",
        providerId,
        sourceKind: "internal",
        updatedAt: null,
        sizeBytes: 512,
      }],
      totalCount: 1,
      totalBytes: 512,
    });
    const cleanupExecute = vi.spyOn(backend, "invalidChildCleanupExecute")
      .mockRejectedValueOnce(new Error("confirm force close and retry"))
      .mockResolvedValueOnce({
        providerId,
        deleted: [{
          threadId: "occupied-child",
          title: "客户端占用的子会话",
          message: "已永久删除子会话",
        }],
        failed: [],
        clientRestarted: true,
        warning: null,
      });

    await wrapper.get('[data-testid="cleanup-child-sessions"]').trigger("click");
    await flushPromises();
    await wrapper.get(".child-cleanup-modal .danger-button").trigger("click");
    await flushPromises();

    expect(wrapper.get(".confirm-modal").text()).toContain("取消清理");
    await wrapper.get(".confirm-modal .danger-button").trigger("click");
    await flushPromises();

    expect(cleanupExecute).toHaveBeenNthCalledWith(
      2,
      workspace.activeProfileId,
      providerId,
      ["occupied-child"],
      true,
    );
    expect(wrapper.find(".confirm-modal").exists()).toBe(false);
    expect(wrapper.get(".child-cleanup-modal").text()).toContain("已删除1");
  });

  it("only selects eligible sessions and opens a replica preview", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();
    const workspace = useWorkspaceStore(pinia);

    const sessionCheckboxes = wrapper.findAll('.session-row input[type="checkbox"]');
    await sessionCheckboxes[0].setValue(true);
    const previewButton = wrapper.find(".sync-pane-actions .primary-button");

    expect(previewButton.attributes("disabled")).toBeUndefined();
    await previewButton.trigger("click");
    await flushPromises();
    const modal = wrapper.get(".preview-modal");
    expect(modal.text()).toContain("会话副本预览");
    expect(modal.text()).toContain("新的 Thread ID");
    expect(modal.find(".target-disclosure").exists()).toBe(false);
    expect(modal.find(".replication-info-note").exists()).toBe(true);
    expect(modal.text()).toContain("复制会保留 OpenAI-API 中的原会话");
    expect(modal.text()).toContain("创建新的独立会话");
    expect(modal.text()).toContain("使用“同步会话”功能在两个会话之间同步更新");
    expect(modal.text()).not.toContain(workspace.activeProfile!.codexHome);
  });

  it("offers an optional Desktop restart after copying without requiring CLI restart", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const source = workspace.selectableSessions[0]!;
    const execute = vi.spyOn(backend, "replicationExecute").mockResolvedValue({
      jobId: "replication-job",
      targetProviderId: workspace.currentProviderId!,
      created: [{
        sourceThreadId: source.threadId,
        replicaThreadId: "replica-thread",
        title: source.title,
        status: "copied",
        message: "已创建副本",
      }],
      skipped: [],
      failed: [],
      clientRestarted: false,
      warning: null,
    });
    const restart = vi.spyOn(backend, "restartCodexClient").mockResolvedValue(true);

    await wrapper.findAll('.session-row input[type="checkbox"]')[0].setValue(true);
    await wrapper.get(".sync-pane-actions .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".preview-modal .primary-button").trigger("click");
    await flushPromises();

    expect(execute).toHaveBeenCalledWith(workspace.activeProfileId, [source.threadId], false);
    const restartModal = wrapper.get(".restart-client-modal");
    expect(restartModal.text()).toContain("是否重启 Codex Desktop");
    expect(restartModal.text()).toContain("如果有正在运行的任务");
    expect(restartModal.text()).toContain("使用 CLI 无需重启");
    expect(restartModal.text()).toContain("codex resume");

    await restartModal.get(".secondary-button").trigger("click");
    expect(restart).not.toHaveBeenCalled();
    expect(wrapper.find(".restart-client-modal").exists()).toBe(false);
  });

  it("restarts Desktop only after explicit confirmation", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const source = workspace.selectableSessions[0]!;
    vi.spyOn(backend, "replicationExecute").mockResolvedValue({
      jobId: "replication-job",
      targetProviderId: workspace.currentProviderId!,
      created: [{
        sourceThreadId: source.threadId,
        replicaThreadId: "replica-thread",
        title: source.title,
        status: "copied",
        message: "已创建副本",
      }],
      skipped: [],
      failed: [],
      clientRestarted: false,
      warning: null,
    });
    const restart = vi.spyOn(backend, "restartCodexClient").mockResolvedValue(true);

    await wrapper.findAll('.session-row input[type="checkbox"]')[0].setValue(true);
    await wrapper.get(".sync-pane-actions .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".preview-modal .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".restart-client-modal .primary-button").trigger("click");
    await flushPromises();

    expect(restart).toHaveBeenCalledWith(workspace.activeProfileId, false);
    expect(wrapper.find(".restart-client-modal").exists()).toBe(false);
    expect(wrapper.get(".toast").text()).toContain("Codex Desktop 已重启");
  });

  it("asks before force-closing Desktop when a normal restart cannot stop it", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const source = workspace.selectableSessions[0]!;
    vi.spyOn(backend, "replicationExecute").mockResolvedValue({
      jobId: "replication-job",
      targetProviderId: workspace.currentProviderId!,
      created: [{
        sourceThreadId: source.threadId,
        replicaThreadId: "replica-thread",
        title: source.title,
        status: "copied",
        message: "已创建副本",
      }],
      skipped: [],
      failed: [],
      clientRestarted: false,
      warning: null,
    });
    const restart = vi.spyOn(backend, "restartCodexClient")
      .mockRejectedValueOnce(new Error("confirm force close and retry"))
      .mockResolvedValueOnce(true);

    await wrapper.findAll('.session-row input[type="checkbox"]')[0].setValue(true);
    await wrapper.get(".sync-pane-actions .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".preview-modal .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".restart-client-modal .primary-button").trigger("click");
    await flushPromises();

    expect(wrapper.get(".confirm-modal").text()).toContain("取消重启");
    await wrapper.get(".confirm-modal .danger-button").trigger("click");
    await flushPromises();

    expect(restart).toHaveBeenNthCalledWith(2, workspace.activeProfileId, true);
    expect(wrapper.find(".confirm-modal").exists()).toBe(false);
    expect(wrapper.find(".restart-client-modal").exists()).toBe(false);
  });

  it("previews migration warnings and migrates selected sessions in place", async () => {
    const pinia = createPinia();
    const wrapper = mount(App, {
      global: { plugins: [pinia] },
    });
    await flushPromises();

    const workspace = useWorkspaceStore(pinia);
    const source = workspace.selectableSessions[0]!;
    await wrapper.findAll('.session-row input[type="checkbox"]')[0].setValue(true);

    const migrationPreview = vi.spyOn(backend, "replicationPreview").mockResolvedValue({
      profileId: workspace.activeProfileId!,
      targetProviderId: workspace.currentProviderId!,
      items: [{
        threadId: source.threadId,
        title: source.title,
        sourceProviderId: source.providerId,
        action: "create_replica",
        reason: `将创建新的 Thread ID 并归入 ${workspace.currentProviderId}`,
        sourceSha256: source.sha256,
        replicaThreadId: null,
        sizeBytes: source.sizeBytes,
      }],
      createCount: 1,
      skipCount: 0,
      invalidCount: 0,
      estimatedBytes: source.sizeBytes,
    });
    const migrate = vi.spyOn(backend, "replicationMigrate").mockResolvedValue({
      jobId: "migration-job",
      targetProviderId: workspace.currentProviderId!,
      created: [{
        sourceThreadId: source.threadId,
        replicaThreadId: "migrated-thread",
        title: source.title,
        status: "migrated",
        message: "已迁移到当前供应商并永久删除来源会话",
      }],
      skipped: [],
      failed: [],
      clientRestarted: false,
      warning: null,
    });

    await wrapper.get(".migration-trigger").trigger("click");
    await flushPromises();

    expect(migrationPreview).toHaveBeenCalledWith(workspace.activeProfileId, [source.threadId]);
    const modal = wrapper.get(".migration-modal");
    expect(modal.text()).toContain("迁移会话1");
    expect(modal.text()).toContain("原会话将被永久删除且不会备份");
    expect(modal.text()).toContain("并显示为“当前”");
    expect(migrate).not.toHaveBeenCalled();

    await modal.get(".danger-button").trigger("click");
    await flushPromises();

    expect(migrate).toHaveBeenCalledWith(
      workspace.activeProfileId,
      [source.threadId],
      false,
    );
    expect(wrapper.find(".migration-modal").exists()).toBe(false);
    expect(wrapper.get(".toast").text()).toContain("迁移完成：已迁移 1 条");
    expect(wrapper.get(".restart-client-modal").text()).toContain("迁移完成");
    expect(wrapper.get(".restart-client-modal").text()).toContain("使用 CLI 无需重启");
  });
});
