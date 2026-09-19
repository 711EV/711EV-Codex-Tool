// @vitest-environment jsdom

import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App.vue";
import { appUpdater } from "./services/appUpdater";
import { backend } from "./services/backend";

async function mountApp() {
  const wrapper = mount(App, { global: { plugins: [createPinia()] } });
  await flushPromises();
  return wrapper;
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("portrait ChatGPT relay workspace", () => {
  it("uses the three-section portrait shell and blocks the WebView context menu", async () => {
    const wrapper = await mountApp();
    expect(wrapper.get(".app-frame").classes()).toContain("app-frame");
    expect(wrapper.find(".desktop-titlebar").exists()).toBe(true);
    expect(wrapper.find(".app-content").exists()).toBe(true);
    expect(wrapper.find(".app-footer").exists()).toBe(true);
    expect(wrapper.find(".sidebar").exists()).toBe(false);

    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    wrapper.get(".app-frame").element.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    wrapper.unmount();
  });

  it("shows branded titlebar controls without GitHub stars", async () => {
    const wrapper = await mountApp();
    expect(wrapper.get(".desktop-titlebar-name").text()).toBe("ChatGPT中转工具");
    expect(wrapper.get(".desktop-titlebar-logo").attributes("src")).toContain("app-icon-light.png");
    expect(wrapper.findAll(".desktop-window-button")).toHaveLength(4);
    expect(wrapper.find('[aria-label="打开交流群"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="打开 GitHub 项目"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="最小化"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="关闭到系统托盘"]').exists()).toBe(true);
    expect(wrapper.get(".desktop-link-controls").findAll("button").map((button) => button.attributes("aria-label")))
      .toEqual(["打开交流群", "打开 GitHub 项目"]);
    expect(wrapper.find(".github-star-count").exists()).toBe(false);
    wrapper.unmount();
  });

  it("renders provider cards with official privacy and relay details", async () => {
    const wrapper = await mountApp();
    const cards = wrapper.findAll(".provider-card");
    expect(cards.length).toBeGreaterThan(1);
    const official = cards.find((card) => card.find(".provider-card-name").text().includes("官方账号"));
    const relay = cards.find((card) => card.text().includes("custom"));
    expect(official).toBeDefined();
    expect(official!.text()).not.toContain("openai");
    expect(official!.text()).toContain("官方");
    expect(official!.text()).toContain("认证状态");
    expect(official!.text()).not.toContain("供应商类型");
    expect(official!.text()).not.toContain("API 地址");
    expect(official!.text()).not.toContain("API 密钥");
    expect(official!.findAll(".card-action-button").map((button) => button.text()))
      .toEqual(["会话清理", "刷新"]);
    expect(relay).toBeDefined();
    expect(relay!.text()).not.toContain("供应商类型");
    expect(relay!.text()).toContain("API 地址");
    expect(relay!.text()).toContain("API 密钥");
    expect(relay!.text()).toContain("正在使用");
    expect(relay!.find(".provider-card-meta").exists()).toBe(true);
    expect(relay!.find(".provider-card-stats").exists()).toBe(true);
    expect(relay!.findAll(".card-action-button").map((button) => button.text()))
      .toEqual(["修改配置", "会话恢复", "会话清理", "刷新"]);
    const inactiveRelay = cards.find((card) => card !== relay && card.text().includes("711EV"));
    if (inactiveRelay) expect(inactiveRelay.findAll(".card-action-button").map((button) => button.text())).not.toContain("会话恢复");
    const unconfigured = cards.find((card) => card.text().includes("SHUAI-API"));
    expect(unconfigured?.findAll(".card-action-button").map((button) => button.text()))
      .toEqual(["添加配置", "会话清理", "刷新"]);
    expect(unconfigured?.find(".card-action-button").find("svg").exists()).toBe(true);
    wrapper.unmount();
  });

  it("shows full relay values and copies the API address and key", async () => {
    const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    try {
      const wrapper = await mountApp();
      const relay = wrapper.findAll(".provider-card").find((card) => card.text().includes("custom"))!;
      const address = relay.get('[aria-label="复制 API 地址"]');
      const apiKey = relay.get('[aria-label="复制 API 密钥"]');
      expect(address.text()).toBe("https://api.example.com/v1");
      expect(apiKey.text()).toBe("sk-demo-provider-key");

      await address.trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith("https://api.example.com/v1");
      expect(wrapper.find(".app-message").text()).toContain("API 地址已复制");

      await apiKey.trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith("sk-demo-provider-key");
      expect(wrapper.find(".app-message").text()).toContain("API 密钥已复制");
      wrapper.unmount();
    } finally {
      if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard);
      else delete (navigator as unknown as { clipboard?: Clipboard }).clipboard;
    }
  });

  it("supports the 711EV add preset and saves the provider draft", async () => {
    const save = vi.spyOn(backend, "providerConfigSave");
    const wrapper = await mountApp();
    await wrapper.get('[data-testid="add-provider"]').trigger("click");
    const modal = wrapper.get(".provider-config-modal");
    const preset = modal.get('[role="switch"]');
    expect(preset.attributes("aria-checked")).toBe("false");
    await preset.trigger("click");
    expect(preset.attributes("aria-checked")).toBe("true");
    expect((modal.findAll("input")[0].element as HTMLInputElement).value).toBe("711EV");
    expect((modal.findAll("input")[1].element as HTMLInputElement).value)
      .toBe("https://ai.711ev.com/v1");
    await modal.findAll("input")[0].setValue("custom-new");
    await modal.findAll("input")[1].setValue("https://relay.example/v1");
    await modal.find('input[type="password"]').setValue("sk-test");
    await modal.get(".primary-button").trigger("click");
    await flushPromises();
    expect(save).toHaveBeenCalledWith(expect.objectContaining({
      providerId: "custom-new",
      baseUrl: "https://relay.example/v1",
      apiKey: "sk-test",
      template: "711ev",
    }));
    expect(wrapper.find(".provider-config-modal").exists()).toBe(false);
    wrapper.unmount();
  });

  it("opens the selected relay provider in the existing edit dialog and refreshes after save", async () => {
    const save = vi.spyOn(backend, "providerConfigSave");
    const refresh = vi.spyOn(backend, "providerWorkspace");
    const wrapper = await mountApp();
    const relay = wrapper.findAll(".provider-card").find((card) => card.text().includes("custom"))!;
    await relay.find(".card-action-button").trigger("click");
    const modal = wrapper.get(".provider-config-modal");
    expect((modal.findAll("input")[0].element as HTMLInputElement).value).toBe("custom");
    expect((modal.findAll("input")[1].element as HTMLInputElement).value)
      .toBe("https://api.example.com/v1");
    expect(modal.get(".secret-toggle").find("svg").exists()).toBe(true);
    await modal.findAll("input")[1].setValue("https://updated.example/v1");
    await modal.get(".primary-button").trigger("click");
    await flushPromises();
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ providerId: "custom", baseUrl: "https://updated.example/v1" }));
    expect(refresh).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps modal state on backdrop clicks and closes the topmost modal with Escape", async () => {
    const wrapper = await mountApp();
    await wrapper.get('[data-testid="add-provider"]').trigger("click");
    await wrapper.get(".modal-backdrop").trigger("mousedown");
    expect(wrapper.find(".provider-config-modal").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(wrapper.find(".provider-config-modal").exists()).toBe(false);
    wrapper.unmount();
  });

  it("groups recovery sessions and omits the provider that opened recovery", async () => {
    const wrapper = await mountApp();
    const relay = wrapper.findAll(".provider-card").find((card) => card.text().includes("custom"))!;
    await relay.findAll(".card-action-button")[1].trigger("click");
    await flushPromises();
    const modal = wrapper.get(".recovery-modal");
    expect(modal.find(".eyebrow").exists()).toBe(false);
    expect(modal.text()).toContain("选择要恢复的会话");
    expect(modal.findAll(".recovery-group").length).toBeGreaterThan(0);
    expect(modal.findAll(".recovery-group").some((group) => group.text().startsWith("custom"))).toBe(false);
    expect(modal.findAll(".recovery-row .action-badge").every((badge) => badge.text() === "主会话"))
      .toBe(true);
    expect(modal.text()).not.toContain("子会话");
    expect(modal.text()).not.toContain("已归档");
    const firstGroup = modal.find(".recovery-group");
    const groupToggle = firstGroup.get(".recovery-group-toggle");
    expect(groupToggle.attributes("aria-expanded")).toBe("true");
    await groupToggle.trigger("click");
    expect(groupToggle.attributes("aria-expanded")).toBe("false");
    expect(firstGroup.findAll(".recovery-row")).toHaveLength(0);
    expect(modal.find(".recovery-search").exists()).toBe(false);
    expect(modal.text()).not.toContain("更新时间");
    expect(modal.text()).not.toContain("来源");
    expect(modal.findAll(".recovery-actions button").map((button) => button.text()))
      .toEqual(["取消", "恢复选中"]);
    wrapper.unmount();
  });

  it("requires explicit close confirmation and limits cleanup to openedProviderId", async () => {
    const childPreview = vi.spyOn(backend, "invalidChildCleanupPreview").mockResolvedValue({
      profileId: "demo-account", providerId: "custom", items: [{
        threadId: "current-2", title: "child", providerId: "custom", sourceKind: "internal", updatedAt: null, sizeBytes: 10,
      }], totalCount: 1, totalBytes: 10,
    });
    const execute = vi.spyOn(backend, "invalidChildCleanupExecute").mockResolvedValue({
      providerId: "custom", deleted: [], failed: [], clientRestarted: false, warning: null,
    });
    const wrapper = await mountApp();
    const relay = wrapper.findAll(".provider-card").find((card) => card.text().includes("custom"))!;
    await relay.findAll(".card-action-button")[2].trigger("click");
    await flushPromises();
    expect(wrapper.find(".recovery-modal").exists()).toBe(true);
    expect(wrapper.get(".recovery-modal").text()).toContain("会话清理");
    expect(wrapper.get(".recovery-modal").findAll(".recovery-actions button").map((button) => button.text()))
      .toEqual(["取消", "删除选中"]);
    await wrapper.get(".recovery-modal .recovery-row input").setValue(true);
    await wrapper.get(".recovery-modal .primary-button").trigger("click");
    await flushPromises();
    expect(childPreview).toHaveBeenCalledWith("demo-account", "custom");
    expect(wrapper.get(".force-close-modal").text()).toContain("需要关闭 ChatGPT");
    expect(wrapper.findAll(".force-close-modal button").map((button) => button.text()))
      .toEqual(["", "取消", "立即关闭"]);
    await wrapper.get(".force-close-modal .secondary-button").trigger("click");
    expect(execute).not.toHaveBeenCalled();
    await wrapper.get(".recovery-modal .primary-button").trigger("click");
    await flushPromises();
    await wrapper.get(".force-close-modal .primary-button").trigger("click");
    await flushPromises();
    expect(execute).toHaveBeenCalledWith("demo-account", "custom", ["current-2"], true);
    wrapper.unmount();
  });

  it("restores selected sessions into the provider that opened recovery", async () => {
    const preview = vi.spyOn(backend, "replicationPreview").mockResolvedValue({
      profileId: "demo-account", targetProviderId: "custom", items: [], createCount: 1, skipCount: 0, invalidCount: 0, estimatedBytes: 10,
    });
    const execute = vi.spyOn(backend, "replicationExecute").mockResolvedValue({
      jobId: "job", targetProviderId: "custom", created: [{ sourceThreadId: "source", replicaThreadId: "copy", title: "source", status: "created", message: "ok" }], skipped: [], failed: [], clientRestarted: false, warning: null,
    });
    const wrapper = await mountApp();
    const relay = wrapper.findAll(".provider-card").find((card) => card.text().includes("custom"))!;
    await relay.findAll(".card-action-button")[1].trigger("click");
    await flushPromises();
    const selectable = wrapper.get(".recovery-modal").findAll(".recovery-row input:not(:disabled)")[0];
    await selectable.setValue(true);
    await wrapper.get(".recovery-modal .primary-button").trigger("click");
    expect(wrapper.get(".force-close-modal").text()).toContain("恢复会话");
    await wrapper.get(".force-close-modal .primary-button").trigger("click");
    await flushPromises();
    expect(preview).toHaveBeenCalledWith("demo-account", ["019b3e6d-demo-1"]);
    expect(execute).toHaveBeenCalledWith("demo-account", ["019b3e6d-demo-1"], expect.any(String), true);
    wrapper.unmount();
  });

  it("places footer links in the requested order and exposes the version tooltip", async () => {
    const wrapper = await mountApp();
    expect(wrapper.get(".app-footer").findAll("button").map((button) => button.text()))
      .toEqual(["使用生图", "推荐梯子", "中转站", "检查更新"]);
    expect(wrapper.get('[data-testid="check-application-update"]').element.parentElement?.textContent)
      .toContain(`当前版本 v${__APP_VERSION__}`);
    wrapper.unmount();
  });

  it("checks updates from the footer and reports the latest version through Message", async () => {
    vi.spyOn(appUpdater, "check").mockResolvedValue(null);
    const wrapper = await mountApp();
    await wrapper.get('[data-testid="check-application-update"]').trigger("click");
    await flushPromises();
    expect(wrapper.find(".app-message").text()).toContain(`当前已是最新版本 ${__APP_VERSION__}`);
    wrapper.unmount();
  });

  it("does not render migration controls", async () => {
    const wrapper = await mountApp();
    expect(wrapper.text()).not.toContain("迁移");
    expect(wrapper.find(".migration-trigger").exists()).toBe(false);
    wrapper.unmount();
  });
});
