import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { verifyMacAppResources, verifyMcpResources } from "./mcp-resources.mjs";
import { verifyMcpProtocol } from "./verify-mcp-protocol.mjs";

assert.equal(process.platform, "darwin", "macOS package verification must run on macOS");
const run = promisify(execFile);
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const bundle = path.resolve(process.argv[2] ?? "src-tauri/target/universal-apple-darwin/release/bundle");
const expected = await verifyMcpResources(path.join(projectRoot, "src-tauri/resources/mcp"), "macos");

async function singleFile(directory, suffix) {
  const matches = (await readdir(directory)).filter((name) => name.endsWith(suffix));
  assert.equal(matches.length, 1, `Expected one ${suffix} in ${directory}`);
  return path.join(directory, matches[0]);
}

async function checkApp(app) {
  const executable = await verifyMacAppResources(app, expected);
  const { stdout } = await run("lipo", ["-archs", executable]);
  assert.deepEqual(stdout.trim().split(/\s+/).sort(), ["arm64", "x86_64"], "Application must be universal");
}

const temporary = await mkdtemp(path.join(tmpdir(), "711ev-macos-package-"));
const mount = path.join(temporary, "dmg");
let mounted = false;
try {
  await checkApp(await singleFile(path.join(bundle, "macos"), ".app"));
  const archive = await singleFile(path.join(bundle, "macos"), ".app.tar.gz");
  const unpacked = path.join(temporary, "updater");
  await mkdir(unpacked);
  await run("tar", ["-xzf", archive, "-C", unpacked]);
  await checkApp(await singleFile(unpacked, ".app"));

  const dmg = await singleFile(path.join(bundle, "dmg"), ".dmg");
  await mkdir(mount);
  await run("hdiutil", ["attach", dmg, "-readonly", "-nobrowse", "-mountpoint", mount]);
  mounted = true;
  await checkApp(await singleFile(mount, ".app"));

  const nativeArchitecture = process.arch === "arm64" ? "arm64" : "amd64";
  assert.ok(["arm64", "x64"].includes(process.arch), "Unsupported macOS runner architecture");
  await verifyMcpProtocol(expected.get(`image-mcp-darwin-${nativeArchitecture}`));
  console.log("macOS 包验证通过：App、DMG、更新包均含双架构 MCP，可执行权限完好，无 Windows 资源。");
} finally {
  // Never recursively remove a directory while a disk image is mounted in it.
  if (mounted) await run("hdiutil", ["detach", mount]);
  await rm(temporary, { recursive: true, force: true });
}
