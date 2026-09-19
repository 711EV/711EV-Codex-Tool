import { spawn } from "node:child_process";
import { access, chmod, mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";
import { verifyMcpResources } from "./mcp-resources.mjs";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const mcpRoot = path.join(projectRoot, "mcp");
const resourceRoot = path.join(projectRoot, "src-tauri", "resources", "mcp");
const requestedPlatform = process.argv.slice(2);
if (requestedPlatform.length && (requestedPlatform.length !== 2 || requestedPlatform[0] !== "--platform")) {
  throw new Error("用法：node scripts/build-mcp.mjs [--platform windows|macos]");
}
const platform = requestedPlatform[1] ?? ({ win32: "windows", darwin: "macos" }[process.platform]);
if (platform !== "windows" && platform !== "macos") throw new Error("MCP 构建只支持 Windows 和 macOS");
const goos = platform === "windows" ? "windows" : "darwin";
const suffix = platform === "windows" ? ".exe" : "";
const outputs = ["amd64", "arm64"].map((architecture) => ({
  architecture,
  name: `image-mcp-${goos}-${architecture}${suffix}`,
}));
const go = await findGo();
await run(go, ["test", "./..."], { CGO_ENABLED: "0" });

// This directory only contains generated MCP resources; never clear a parent.
if (path.resolve(resourceRoot) !== path.join(projectRoot, "src-tauri", "resources", "mcp")) {
  throw new Error("拒绝清理非 MCP 资源目录");
}
await rm(resourceRoot, { recursive: true, force: true });
await mkdir(resourceRoot, { recursive: true });
for (const output of outputs) {
  const destination = path.join(resourceRoot, output.name);
  await run(go, ["build", "-trimpath", "-ldflags=-s -w", "-o", destination, "."], {
    CGO_ENABLED: "0", GOOS: goos, GOARCH: output.architecture,
  });
  if (platform === "macos") await chmod(destination, 0o755);
}
// A cross-build on Windows cannot validate POSIX permissions; the macOS runner does.
await verifyMcpResources(resourceRoot, platform, platform === "macos" && process.platform !== "win32");
console.log(`MCP ${platform} 双架构资源已验证：${outputs.map(({ name }) => name).join("、")}`);

async function findGo() {
  if (process.env.GO_BINARY) return process.env.GO_BINARY;
  if (process.platform === "win32") {
    const portable = path.resolve(projectRoot, "..", "go-portable", "go", "bin", "go.exe");
    try { await access(portable); return portable; } catch { /* Use PATH on other machines. */ }
  }
  return "go";
}

function run(command, args, overrides) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: mcpRoot, stdio: "inherit", env: { ...process.env, ...overrides }, windowsHide: true });
    child.on("error", reject);
    child.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`Go 执行失败，退出码：${code}`)));
  });
}
