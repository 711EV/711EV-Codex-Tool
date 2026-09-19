import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { imageMcpService } from "./imageMcpService";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
afterEach(() => vi.resetAllMocks());

describe("image MCP IPC", () => {
  it("passes the selected profile to every command", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    await imageMcpService.ensureInstalled("first");
    expect(invoke).toHaveBeenLastCalledWith("ensure_image_mcp", { profileId: "first" });
    await imageMcpService.refreshIfInstalled("second");
    expect(invoke).toHaveBeenLastCalledWith("refresh_image_mcp_if_installed", { profileId: "second" });
    await imageMcpService.repair("third");
    expect(invoke).toHaveBeenLastCalledWith("repair_image_mcp", { profileId: "third" });
  });

  it("does not install or repair from browser preview", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    expect(await imageMcpService.refreshIfInstalled("first")).toBeNull();
    await expect(imageMcpService.ensureInstalled("first")).rejects.toThrow("浏览器预览不提供图片工具");
    await expect(imageMcpService.repair("first")).rejects.toThrow("浏览器预览不执行图片工具修复");
    expect(invoke).not.toHaveBeenCalled();
  });
});
