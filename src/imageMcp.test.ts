// @vitest-environment jsdom
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia } from "pinia";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.vue";
import { backend } from "./services/backend";
import { imageMcpService, type ImageMcpStatus } from "./services/imageMcpService";
import { useWorkspaceStore } from "./stores/workspace";

const installation = { path: "test/image-mcp.exe", platform: "windows", architecture: "amd64", installed: false };
const idle: ImageMcpStatus = { ...installation, eligible: true, configRegistered: false, agentsConfigured: false };
const ready: ImageMcpStatus = { ...idle, configRegistered: true, agentsConfigured: true };
let wrapper: VueWrapper | undefined;

async function mountApp() {
  const pinia = createPinia();
  wrapper = mount(App, { global: { plugins: [pinia] } });
  await flushPromises();
  return { wrapper, workspace: useWorkspaceStore(pinia) };
}

beforeEach(() => {
  vi.spyOn(imageMcpService, "refreshIfInstalled").mockResolvedValue(idle);
  vi.spyOn(imageMcpService, "ensureInstalled").mockResolvedValue(installation);
  vi.spyOn(imageMcpService, "repair").mockResolvedValue(installation);
  vi.spyOn(backend, "restartCodexClient").mockResolvedValue(true);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.restoreAllMocks();
});

describe("image MCP integration", () => {
  it("replaces About with image setup and preserves setup when restart is cancelled", async () => {
    const { wrapper, workspace } = await mountApp();
    expect(wrapper.get(".app-footer").text()).not.toContain("关于我们");
    expect(wrapper.find('[aria-label="打开 711EV 导航"]').exists()).toBe(false);
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    await flushPromises();
    expect(imageMcpService.ensureInstalled).toHaveBeenCalledWith(workspace.activeProfileId);
    expect(wrapper.get('[role="dialog"]').text()).toContain("重启 ChatGPT 后生图功能才能生效。");
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("生图正常");
    await wrapper.get('[role="dialog"] .secondary-button').trigger("click");
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(backend.restartCodexClient).not.toHaveBeenCalled();
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    expect(imageMcpService.ensureInstalled).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("生图功能正常");
  });

  it("restarts only after confirmation and retains the dialog on restart failure", async () => {
    const { wrapper, workspace } = await mountApp();
    vi.mocked(backend.restartCodexClient).mockRejectedValueOnce(new Error("启动失败"));
    const profileId = workspace.activeProfileId;
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    await flushPromises();
    expect(backend.restartCodexClient).not.toHaveBeenCalled();
    await wrapper.get('[role="dialog"] .primary-button').trigger("click");
    await flushPromises();
    expect(backend.restartCodexClient).toHaveBeenCalledWith(profileId);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("启动失败");
    await wrapper.get('[role="dialog"] .primary-button').trigger("click");
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it("requires confirmation to repair and reports errors without a false success", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue({ ...idle, error: "文件占用" });
    const { wrapper, workspace } = await mountApp();
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("修复生图");
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    expect(wrapper.get('[role="dialog"]').text()).toContain("修复生图会关闭 ChatGPT 客户端。");
    expect(imageMcpService.repair).not.toHaveBeenCalled();
    await wrapper.get('[role="dialog"] .secondary-button').trigger("click");
    expect(imageMcpService.repair).not.toHaveBeenCalled();
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    vi.mocked(imageMcpService.repair).mockRejectedValueOnce(new Error("无法启动 ChatGPT"));
    await wrapper.get('[role="dialog"] .primary-button').trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("无法启动 ChatGPT");
    expect(wrapper.text()).not.toContain("生图功能修复完毕");
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("修复生图");
    await wrapper.get('[role="dialog"] .primary-button').trigger("click");
    await flushPromises();
    expect(imageMcpService.repair).toHaveBeenLastCalledWith(workspace.activeProfileId);
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("生图正常");
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("文件占用");
  });

  it("disables duplicate installation and directory changes while installing", async () => {
    let finish!: (result: typeof installation) => void;
    vi.mocked(imageMcpService.ensureInstalled).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const { wrapper } = await mountApp();
    const button = wrapper.get('[data-testid="image-mcp"]');
    await button.trigger("click");
    await button.trigger("click");
    expect(imageMcpService.ensureInstalled).toHaveBeenCalledTimes(1);
    expect(button.text()).toBe("正在准备");
    expect(wrapper.text()).not.toContain("生图功能仅支持711EV使用");
    expect(button.attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="storage-location-switch"]').attributes("disabled")).toBeDefined();
    finish(installation);
    await flushPromises();
    expect(button.attributes("disabled")).toBeUndefined();
  });

  it("keeps a ready installation ready after startup without opening a dialog", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue(ready);
    const { wrapper } = await mountApp();
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("生图正常");
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("生图功能仅支持711EV使用");
    expect(wrapper.text()).not.toContain("图片工具已更新");
    expect(imageMcpService.ensureInstalled).not.toHaveBeenCalled();
  });

  it("does not let a stale directory refresh overwrite the new directory state", async () => {
    let finish!: (status: ImageMcpStatus) => void;
    const { wrapper, workspace } = await mountApp();
    vi.mocked(imageMcpService.refreshIfInstalled)
      .mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }))
      .mockResolvedValue(idle);
    workspace.providerBuckets = workspace.providerBuckets.map((bucket, index) => ({ ...bucket, isCurrent: index === 0 }));
    await nextTick();
    const other = workspace.profiles.find((profile) => profile.id !== workspace.activeProfileId)!;
    workspace.activeProfileId = other.id;
    await nextTick();
    await flushPromises();
    finish(ready);
    await flushPromises();
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("使用生图");
    expect(imageMcpService.refreshIfInstalled).toHaveBeenLastCalledWith(other.id);
  });

  it("does not show a completed installation dialog for a different directory", async () => {
    let finish!: (result: typeof installation) => void;
    vi.mocked(imageMcpService.ensureInstalled).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const { wrapper, workspace } = await mountApp();
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    workspace.activeProfileId = workspace.profiles.find((profile) => profile.id !== workspace.activeProfileId)!.id;
    await nextTick();
    finish(installation);
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("使用生图");
    expect(backend.restartCodexClient).not.toHaveBeenCalled();
  });

  it("disables image actions when the applied provider is not eligible", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue({ ...ready, eligible: false });
    const { wrapper, workspace } = await mountApp();
    const button = wrapper.get('[data-testid="image-mcp"]');
    expect(button.text()).toBe("使用生图");
    expect(button.attributes("disabled")).toBeDefined();
    const tooltip = wrapper.get('.app-footer > .app-tooltip-trigger:first-child');
    expect(tooltip.get('[role="tooltip"]').text()).toBe("生图功能仅支持711EV使用");
    expect(tooltip.attributes("tabindex")).toBe("0");
    expect(tooltip.attributes("aria-describedby")).toBe(tooltip.get('[role="tooltip"]').attributes("id"));
    await button.trigger("click");
    expect(imageMcpService.ensureInstalled).not.toHaveBeenCalled();
    await workspace.selectProvider(workspace.providerBuckets.find((provider) => !provider.isCurrent)!.providerId);
    await flushPromises();
    expect(button.attributes("disabled")).toBeDefined();
  });

  it("preserves a ready MCP between eligible providers but requires setup after returning", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue(ready);
    const { wrapper, workspace } = await mountApp();
    const current = workspace.providerBuckets.find((bucket) => bucket.isCurrent)!.providerId;
    const other = workspace.providerBuckets.find((bucket) => !bucket.isCurrent)!.providerId;
    const changeProvider = async (providerId: string) => {
      workspace.providerBuckets = workspace.providerBuckets.map((bucket) => ({ ...bucket, isCurrent: bucket.providerId === providerId }));
      await nextTick();
      await flushPromises();
    };
    const button = wrapper.get('[data-testid="image-mcp"]');
    await changeProvider(other);
    expect(button.text()).toBe("生图正常");
    expect(imageMcpService.ensureInstalled).not.toHaveBeenCalled();
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue({ ...idle, eligible: false });
    await changeProvider(current);
    expect(button.attributes("disabled")).toBeDefined();
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue(idle);
    await changeProvider(other);
    expect(button.text()).toBe("使用生图");
    expect(button.attributes("disabled")).toBeUndefined();
    expect(wrapper.text()).not.toContain("生图功能仅支持711EV使用");
    expect(imageMcpService.ensureInstalled).not.toHaveBeenCalled();
    await button.trigger("click");
    await flushPromises();
    expect(imageMcpService.ensureInstalled).toHaveBeenCalledTimes(1);
  });

  it("does not show a provider restriction tooltip when status inspection fails", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockRejectedValue(new Error("无法读取配置"));
    const { wrapper } = await mountApp();
    expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).not.toContain("生图功能仅支持711EV使用");
    const trigger = wrapper.get('.app-footer > .app-tooltip-trigger:first-child');
    expect(trigger.find('[role="tooltip"]').exists()).toBe(false);
    expect(trigger.attributes("tabindex")).toBeUndefined();
    expect(trigger.attributes("aria-describedby")).toBeUndefined();
  });

  it("ignores stale refreshes and pending dialogs after a provider switch", async () => {
    const { wrapper, workspace } = await mountApp();
    await wrapper.get('[data-testid="image-mcp"]').trigger("click");
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    let finish!: (status: ImageMcpStatus) => void;
    vi.mocked(imageMcpService.refreshIfInstalled).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const current = workspace.providerBuckets.find((bucket) => bucket.isCurrent)!.providerId;
    workspace.providerBuckets = workspace.providerBuckets.map((bucket) => ({ ...bucket, isCurrent: bucket.providerId !== current }));
    await nextTick();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    workspace.providerConfigSwitching = true;
    await nextTick();
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue({ ...idle, eligible: false });
    workspace.providerConfigSwitching = false;
    await nextTick();
    await flushPromises();
    finish(ready);
    await flushPromises();
    expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="image-mcp"]').text()).toBe("使用生图");
    expect(backend.restartCodexClient).not.toHaveBeenCalled();
  });

  it("does not undo cleanup when the provider-switch restart prompt is cancelled", async () => {
    vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue(ready);
    const { wrapper, workspace } = await mountApp();
    const switchProvider = vi.spyOn(workspace, "switchProvider").mockImplementation(async (providerId) => {
      workspace.providerConfigSwitching = true;
      await nextTick();
      vi.mocked(imageMcpService.refreshIfInstalled).mockResolvedValue({ ...idle, eligible: false });
      workspace.providerBuckets = workspace.providerBuckets.map((bucket) => ({ ...bucket, isCurrent: bucket.providerId === providerId }));
      workspace.providerConfigSwitching = false;
      return { profileId: workspace.activeProfileId!, providerId, configFile: "config.toml", authFile: "auth.json", restarted: false, warning: null };
    });
    const button = wrapper.findAll('.provider-use-button').find((item) => item.attributes("disabled") === undefined)!;
    await button.trigger("click");
    await flushPromises();
    await vi.waitFor(() => expect(switchProvider).toHaveBeenCalledTimes(1));
    await flushPromises();
    expect(wrapper.get('[role="dialog"]').text()).toContain("是否重启 ChatGPT");
    await wrapper.get('[role="dialog"] .secondary-button').trigger("click");
    expect(wrapper.get('[data-testid="image-mcp"]').attributes("disabled")).toBeDefined();
    expect(backend.restartCodexClient).not.toHaveBeenCalled();
    expect(imageMcpService.ensureInstalled).not.toHaveBeenCalled();
  });
});
