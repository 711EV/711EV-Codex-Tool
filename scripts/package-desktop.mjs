import { cp, mkdir, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";
import packageJson from "../package.json" with { type: "json" };

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const targetRoot = path.join(projectRoot, "src-tauri", "target", "release");
const versionRoot = path.join(projectRoot, "dist", packageJson.version);

const platform = process.platform;
if (platform !== "win32" && platform !== "darwin") {
  throw new Error("桌面发布目前只支持 Windows 和 macOS");
}

const system = platform === "win32" ? "windows" : "macos";
const outputDir = path.join(versionRoot, system);
await rm(outputDir, { recursive: true, force: true });
await mkdir(outputDir, { recursive: true });

if (platform === "win32") {
  await runTauri(["build", "--no-bundle"]);
  await cp(
    path.join(targetRoot, "codex-local-sync.exe"),
    path.join(outputDir, "CodexLocalSync.exe"),
  );
} else {
  await runTauri(["build", "--bundles", "app"]);
  await cp(
    path.join(targetRoot, "bundle", "macos", "Codex Local Sync.app"),
    path.join(outputDir, "Codex Local Sync.app"),
    { recursive: true },
  );
}

console.log(`桌面客户端已生成：${path.relative(projectRoot, outputDir)}`);

function runTauri(args) {
  const executable = platform === "win32" ? "npx.cmd" : "npx";
  return new Promise((resolve, reject) => {
    const child = spawn(executable, ["tauri", ...args], {
      cwd: projectRoot,
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`Tauri 打包失败，退出码：${code ?? "unknown"}`));
    });
  });
}
