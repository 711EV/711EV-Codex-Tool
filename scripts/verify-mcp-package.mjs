import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { verifyMcpResources } from "./mcp-resources.mjs";
import { verifyMcpProtocol } from "./verify-mcp-protocol.mjs";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const executable = path.resolve(process.argv[2] ?? "dist/711EV-Codex-Tool.exe");
const resources = path.join(projectRoot, "src-tauri", "resources", "mcp");
const resourceBinaries = await verifyMcpResources(resources, "windows");
const packaged = await readFile(executable);
const embedded = new Map();
for (const [name, binary] of resourceBinaries) {
  const offset = packaged.indexOf(binary);
  assert.ok(offset >= 0, `${name} is missing from the standalone application`);
  embedded.set(name, packaged.subarray(offset, offset + binary.length));
}
for (const name of ["image-mcp-darwin-amd64", "image-mcp-darwin-arm64"]) {
  assert.equal(packaged.includes(Buffer.from(name)), false, `${name} leaked into Windows package`);
}

const temporary = await mkdtemp(path.join(tmpdir(), "711ev-mcp-check-"));
try {
  if (process.argv[3]) {
    let installer = path.resolve(process.argv[3]);
    if ((await stat(installer)).isDirectory()) {
      const installers = (await readdir(installer)).filter((name) => name.endsWith("-setup.exe"));
      assert.equal(installers.length, 1, "Expected one NSIS installer");
      installer = path.join(installer, installers[0]);
    }
    const extracted = path.join(temporary, "installer");
    await new Promise((resolve, reject) => {
      const extractor = spawn(process.env.SEVEN_ZIP_BINARY ?? "7z", ["x", "-y", `-o${extracted}`, installer], { stdio: "pipe", windowsHide: true });
      let output = "";
      extractor.stdout.on("data", (data) => { output += data; });
      extractor.stderr.on("data", (data) => { output += data; });
      extractor.on("error", reject);
      extractor.on("exit", (code) => code === 0 ? resolve() : reject(new Error(`Installer extraction failed (${code}): ${output}`)));
    });
    const files = await readdir(extracted, { recursive: true });
    const applications = files.filter((name) => path.basename(name) === "711EV-Codex-Tool.exe");
    assert.equal(applications.length, 1, "Installer must contain one application");
    const installedApplication = await readFile(path.join(extracted, applications[0]));
    // Tauri patches only this bundle-kind marker while building the NSIS payload.
    const portableMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
    const installerMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS");
    const markerOffset = packaged.indexOf(portableMarker);
    assert.ok(markerOffset >= 0, "Portable Tauri bundle marker is missing");
    assert.equal(packaged.lastIndexOf(portableMarker), markerOffset, "Portable Tauri bundle marker must be unique");
    assert.deepEqual(installedApplication.subarray(markerOffset, markerOffset + installerMarker.length), installerMarker);
    const expectedApplication = Buffer.from(packaged);
    installerMarker.copy(expectedApplication, markerOffset);
    assert.deepEqual(installedApplication, expectedApplication);
    assert.ok(files.every((name) => !path.basename(name).startsWith("image-mcp-")), "Installer must not duplicate platform MCP resources");
    console.log("NSIS 安装包解包验证通过：除 Tauri 安装类型标记外与便携版一致，无多余 MCP 资源。");
  }
  const nativeArchitecture = /ARM64/i.test(process.env.PROCESSOR_ARCHITEW6432 ?? process.env.PROCESSOR_ARCHITECTURE ?? "") ? "arm64" : "amd64";
  await verifyMcpProtocol(embedded.get(`image-mcp-windows-${nativeArchitecture}.exe`));
  console.log("Windows 包验证通过：内嵌双架构 MCP、无 macOS 资源。");
} finally {
  await rm(temporary, { recursive: true, force: true });
}
