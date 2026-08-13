// @vitest-environment jsdom

import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { describe, expect, it } from "vitest";
import App from "./App.vue";

describe("Codex Local Sync workspace", () => {
  it("loads local profiles and sessions in browser preview mode", async () => {
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("当前登录账号");
    expect(wrapper.text()).toContain("实现登录态切换与会话恢复");
    expect(wrapper.text()).toContain("仅处理本地文件");
  });

  it("enables sync preview after selecting a session", async () => {
    const wrapper = mount(App, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    const sessionCheckboxes = wrapper.findAll('.session-row input[type="checkbox"]');
    await sessionCheckboxes[0].setValue(true);
    const previewButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("预览同步"));

    expect(previewButton?.attributes("disabled")).toBeUndefined();
  });
});
