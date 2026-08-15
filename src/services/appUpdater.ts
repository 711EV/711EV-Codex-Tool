import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import type { ApplicationUpdate, UpdateDownloadProgress } from "../types";

let pendingUpdate: Update | null = null;

export const appUpdater = {
  async check(): Promise<ApplicationUpdate | null> {
    if (!isTauri()) return null;

    if (pendingUpdate) {
      await pendingUpdate.close();
      pendingUpdate = null;
    }

    pendingUpdate = await check({ timeout: 15_000 });
    if (!pendingUpdate) return null;

    return {
      currentVersion: pendingUpdate.currentVersion,
      version: pendingUpdate.version,
      date: pendingUpdate.date ?? null,
      body: pendingUpdate.body ?? null,
    };
  },

  async install(
    onProgress: (progress: UpdateDownloadProgress) => void,
  ): Promise<void> {
    const update = pendingUpdate;
    if (!update) throw new Error("请先检查可用更新");

    let totalBytes: number | null = null;
    let downloadedBytes = 0;
    const report = (event: DownloadEvent) => {
      if (event.event === "Started") {
        totalBytes = event.data.contentLength ?? null;
        downloadedBytes = 0;
      } else if (event.event === "Progress") {
        downloadedBytes += event.data.chunkLength;
      }
      onProgress({
        downloadedBytes,
        totalBytes,
        percent: totalBytes && totalBytes > 0
          ? Math.min(100, Math.round((downloadedBytes / totalBytes) * 100))
          : null,
      });
    };

    await update.downloadAndInstall(report, { timeout: 120_000 });
    pendingUpdate = null;
    await relaunch();
  },

  async clear(): Promise<void> {
    if (!pendingUpdate) return;
    await pendingUpdate.close();
    pendingUpdate = null;
  },
};
