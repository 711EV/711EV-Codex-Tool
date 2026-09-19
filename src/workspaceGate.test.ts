// @vitest-environment jsdom
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia } from "pinia";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as tauri from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import App from "./App.vue";
import WorkspaceGate from "./components/WorkspaceGate.vue";
import { backend } from "./services/backend";
import { imageMcpService } from "./services/imageMcpService";
import { useWorkspaceStore } from "./stores/workspace";
import type { AppState, DiscoveryReport, Profile } from "./types";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(),
  isTauri: vi.fn(() => false),
}));

let wrapper: VueWrapper | undefined;
let state: AppState;
let profiles: Profile[];

function report(items: Profile[] = profiles): DiscoveryReport {
  return { candidatesScanned: 2, discoveredCount: items.length, addedCount: items.length,
    refreshedCount: 0, removedCount: 0, unavailableCount: 0, profiles: items };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function mountApp(selectedProfileId?: string) {
  const pinia = createPinia();
  const workspace = useWorkspaceStore(pinia);
  workspace.activeProfileId = selectedProfileId ?? null;
  wrapper = mount(App, { global: { plugins: [pinia] } });
  await flushPromises();
  return { wrapper, workspace };
}

beforeEach(async () => {
  vi.mocked(tauri.isTauri).mockReturnValue(false);
  state = structuredClone(await backend.getAppState());
  profiles = state.profiles;
  vi.spyOn(backend, "getAppState").mockResolvedValue(state);
  vi.spyOn(backend, "discoverProfiles").mockResolvedValue(report());
  vi.spyOn(backend, "providerConfigSave");
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe("workspace initialization gate", () => {
  it("scans on startup and enters a discovered directory even without a running client", async () => {
    vi.mocked(backend.getAppState).mockResolvedValue({ ...state, profiles: [] });
    const { wrapper, workspace } = await mountApp();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(1);
    expect(workspace.activeProfileId).toBe(profiles[0].id);
    expect(workspace.activeProfile?.appPath).toBeNull();
    expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
    expect(wrapper.findAll(".provider-card").length).toBeGreaterThan(0);
    expect(wrapper.find('[data-testid="add-provider"]').exists()).toBe(true);
  });

  it.each(["missing", "unavailable"])("blocks %s directories without exposing content controls", async (scenario) => {
    vi.mocked(backend.discoverProfiles).mockResolvedValue(report(scenario === "missing" ? [] :
      profiles.map((profile) => ({ ...profile, discoveryState: "unavailable" }))));
    const { wrapper, workspace } = await mountApp();
    expect(workspace.activeProfileId).toBeNull();
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
    expect(wrapper.text()).toContain("未检测到 ChatGPT 客户端配置");
    expect(wrapper.text()).toContain("请先安装并启动 ChatGPT 客户端，再点击「重新检测」。");
    expect(wrapper.find(".storage-bar").exists()).toBe(false);
    expect(wrapper.find(".provider-cards").exists()).toBe(false);
    expect(wrapper.find('[data-testid="add-provider"]').exists()).toBe(false);
    expect(wrapper.get(".app-content").findAll("button").map((button) => button.text()))
      .toEqual(["重新检测", "下载 ChatGPT"]);
    expect(wrapper.get(".desktop-titlebar").element.parentElement).toBe(wrapper.get(".app-frame").element);
    expect(wrapper.get(".app-footer").element.parentElement).toBe(wrapper.get(".app-frame").element);
    expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
    for (const key of ["Escape", "Tab", "Enter"]) {
      window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      await nextTick();
      expect(wrapper.find(".provider-config-modal").exists()).toBe(false);
      expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(true);
    }
    expect(backend.providerConfigSave).not.toHaveBeenCalled();
  });

  it("keeps the checking mask until detection and provider detail loading finish", async () => {
    const detection = deferred<DiscoveryReport>();
    vi.mocked(backend.discoverProfiles).mockReturnValue(detection.promise);
    const { wrapper } = await mountApp();
    const gate = wrapper.getComponent(WorkspaceGate);
    expect(gate.attributes("data-state")).toBe("checking");
    expect(wrapper.get(".app-content").attributes("aria-busy")).toBe("true");
    expect(wrapper.find('[data-testid="add-provider"]').exists()).toBe(false);
    gate.vm.$emit("retry");
    gate.vm.$emit("retry");
    await flushPromises();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(1);

    const detail = deferred<Awaited<ReturnType<typeof backend.providerConfigRead>>>();
    const originalRead = backend.providerConfigRead.bind(backend);
    vi.spyOn(backend, "providerConfigRead").mockImplementation((profileId, providerId) =>
      providerId === "SHUAI-API" ? detail.promise : originalRead(profileId, providerId));
    detection.resolve(report());
    await flushPromises();
    expect(gate.attributes("data-state")).toBe("checking");
    detail.resolve(await originalRead(profiles[0].id, "SHUAI-API"));
    await flushPromises();
    expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
  });

  it("retries a real discovery, stays blocked when empty and enters after a directory appears", async () => {
    vi.mocked(backend.discoverProfiles).mockResolvedValue(report([]));
    const { wrapper, workspace } = await mountApp();
    await wrapper.get('[data-testid="retry-workspace"]').trigger("click");
    await flushPromises();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(2);
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
    vi.mocked(backend.discoverProfiles).mockResolvedValue(report());
    await wrapper.get('[data-testid="retry-workspace"]').trigger("click");
    await flushPromises();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(3);
    expect(workspace.activeProfileId).toBe(profiles[0].id);
    expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
  });

  it.each(["getAppState", "discoverProfiles", "providerWorkspace", "providerConfigRead"] as const)(
    "holds failures from %s separately from ordinary errors and allows recovery", async (method) => {
      vi.spyOn(backend, method).mockRejectedValueOnce(new Error("配置读取失败：测试权限错误"));
      const { wrapper, workspace } = await mountApp();
      expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("failed");
      expect(wrapper.text()).toContain("配置检测失败");
      expect(wrapper.text()).toContain("配置读取失败：测试权限错误");
      expect(wrapper.text()).not.toContain("请先安装并启动");
      expect(wrapper.find('[data-testid="download-client"]').exists()).toBe(false);
      expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
      workspace.error = null;
      await nextTick();
      expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("failed");
      expect(wrapper.text()).toContain("配置读取失败：测试权限错误");
      await wrapper.get('[data-testid="retry-workspace"]').trigger("click");
      await flushPromises();
      expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
    },
  );

  it("blocks a provider detail failure rather than treating it as an unconfigured supplier", async () => {
    vi.spyOn(imageMcpService, "refreshIfInstalled").mockResolvedValue({
      path: "test/image-mcp.exe", platform: "windows", architecture: "amd64", installed: true,
      eligible: true, configRegistered: true, agentsConfigured: true,
    });
    const original = backend.providerConfigRead.bind(backend);
    vi.spyOn(backend, "providerConfigRead").mockImplementation((profileId, providerId) => {
      if (providerId === "SHUAI-API") return Promise.reject(new Error("供应商详情读取失败"));
      return original(profileId, providerId);
    });
    const { wrapper } = await mountApp();
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("failed");
    expect(wrapper.text()).toContain("供应商详情读取失败");
    expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).not.toContain("生图功能仅支持711EV使用");
  });

  it("deduplicates retry clicks until the pending discovery completes", async () => {
    vi.mocked(backend.discoverProfiles).mockResolvedValueOnce(report([]));
    const { wrapper } = await mountApp();
    const detection = deferred<DiscoveryReport>();
    vi.mocked(backend.discoverProfiles).mockReturnValueOnce(detection.promise);
    const gate = wrapper.getComponent(WorkspaceGate);
    await wrapper.get('[data-testid="retry-workspace"]').trigger("click");
    gate.vm.$emit("retry");
    gate.vm.$emit("retry");
    await flushPromises();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(2);
    expect(gate.attributes("data-state")).toBe("checking");
    expect(wrapper.find('[data-testid="retry-workspace"]').exists()).toBe(false);
    detection.resolve(report());
    await flushPromises();
    expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
  });

  it.each([true, false])("retains a valid selection or replaces a stale one: %s", async (valid) => {
    const { workspace } = await mountApp(valid ? profiles[1].id : "removed-profile");
    expect(workspace.activeProfileId).toBe(valid ? profiles[1].id : profiles[0].id);
  });

  it("does not rediscover on focus or release a missing gate on external store changes", async () => {
    vi.mocked(backend.discoverProfiles).mockResolvedValue(report([]));
    const { wrapper, workspace } = await mountApp();
    workspace.appState = { ...state, profiles };
    workspace.activeProfileId = profiles[0].id;
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(backend.discoverProfiles).toHaveBeenCalledTimes(1);
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
  });

  it("opens the confirmed download URL without dismissing the gate and reports opener failures", async () => {
    vi.mocked(backend.discoverProfiles).mockResolvedValue(report([]));
    const { wrapper } = await mountApp();
    vi.mocked(tauri.isTauri).mockReturnValue(true);
    vi.mocked(openUrl).mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("无法启动浏览器"));
    await wrapper.get('[data-testid="download-client"]').trigger("click");
    await flushPromises();
    expect(openUrl).toHaveBeenCalledWith("https://openai.com/zh-Hans-CN/codex/");
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
    await wrapper.get('[data-testid="download-client"]').trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("无法打开链接：无法启动浏览器");
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
  });

  it("keeps normal refresh errors as inline errors after successful initialization", async () => {
    const { wrapper } = await mountApp();
    vi.spyOn(backend, "providerWorkspace").mockRejectedValue(new Error("普通刷新失败"));
    await wrapper.get('[data-testid="rediscover-providers"]').trigger("click");
    await flushPromises();
    expect(wrapper.get(".inline-error").text()).toContain("普通刷新失败");
    expect(wrapper.find('[data-testid="workspace-gate"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="add-provider"]').exists()).toBe(true);
  });

  it("discards initialization completion after unmount before loading provider details", async () => {
    const detection = deferred<DiscoveryReport>();
    vi.mocked(backend.discoverProfiles).mockReturnValue(detection.promise);
    const config = vi.spyOn(backend, "providerConfigRead");
    const { wrapper } = await mountApp();
    wrapper.unmount();
    detection.resolve(report());
    await flushPromises();
    // Store discovery loads the selected supplier, but the disposed UI must not start the detail pass.
    expect(config).toHaveBeenCalledTimes(1);
  });
});

describe("provider form guards", () => {
  it("reports an empty name only when a valid directory is selected", async () => {
    const { wrapper } = await mountApp();
    await wrapper.get('[data-testid="add-provider"]').trigger("click");
    await wrapper.get(".provider-config-modal .primary-button").trigger("click");
    expect(wrapper.text()).toContain("请填写供应商名称");
    expect(backend.providerConfigSave).not.toHaveBeenCalled();
  });

  it("saves the unedited 711EV preset with its captured directory", async () => {
    const { wrapper, workspace } = await mountApp();
    await wrapper.get('[data-testid="add-provider"]').trigger("click");
    await wrapper.get('.provider-config-modal [role="switch"]').trigger("click");
    await wrapper.get('.provider-config-modal input[type="password"]').setValue("sk-test-fake-key");
    await wrapper.get(".provider-config-modal .primary-button").trigger("click");
    await flushPromises();
    expect(backend.providerConfigSave).toHaveBeenCalledWith(expect.objectContaining({
      profileId: workspace.activeProfileId, providerId: "711EV", baseUrl: "https://ai.711ev.com/v1", template: "711ev",
    }));
    expect(wrapper.text()).not.toContain("请填写供应商名称");
  });

  it("rejects stale open/save handlers after their directory disappears", async () => {
    const { wrapper, workspace } = await mountApp();
    const add = wrapper.get('[data-testid="add-provider"]');
    await add.trigger("click");
    await wrapper.get('.provider-config-modal [role="switch"]').trigger("click");
    const save = wrapper.get(".provider-config-modal .primary-button");
    workspace.appState = { ...state, profiles: [] };
    await save.trigger("click");
    await add.trigger("click");
    expect(wrapper.text()).toContain("未选择配置目录");
    expect(wrapper.text()).not.toContain("请填写供应商名称");
    expect(wrapper.find(".provider-config-modal").exists()).toBe(false);
    expect(backend.providerConfigSave).not.toHaveBeenCalled();
    expect(wrapper.get('[data-testid="workspace-gate"]').attributes("data-state")).toBe("missing");
  });
});
