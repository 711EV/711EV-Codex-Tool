import { invoke, isTauri } from "@tauri-apps/api/core";

export interface ImageMcpInstallation {
  path: string;
  platform: string;
  architecture: string;
  installed: boolean;
}

export interface ImageMcpStatus extends ImageMcpInstallation {
  eligible: boolean;
  error?: string | null;
  configRegistered: boolean;
  agentsConfigured: boolean;
}

export function isImageMcpReady(status: ImageMcpStatus | null): boolean {
  return Boolean(status?.eligible && !status.error && status.configRegistered && status.agentsConfigured);
}

export const imageMcpService = {
  async ensureInstalled(profileId: string): Promise<ImageMcpInstallation> {
    if (!isTauri()) throw new Error("浏览器预览不提供图片工具");
    return invoke("ensure_image_mcp", { profileId });
  },
  async refreshIfInstalled(profileId: string): Promise<ImageMcpStatus | null> {
    if (!isTauri()) return null;
    return invoke("refresh_image_mcp_if_installed", { profileId });
  },
  async repair(profileId: string): Promise<ImageMcpInstallation> {
    if (!isTauri()) throw new Error("浏览器预览不执行图片工具修复");
    return invoke("repair_image_mcp", { profileId });
  },
};
