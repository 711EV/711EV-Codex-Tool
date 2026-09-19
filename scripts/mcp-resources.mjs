import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import path from "node:path";

export function mcpResourceNames(platform) {
  assert.ok(["windows", "macos"].includes(platform), `Unsupported MCP platform: ${platform}`);
  return ["amd64", "arm64"].map((arch) => platform === "windows"
    ? `image-mcp-windows-${arch}.exe` : `image-mcp-darwin-${arch}`);
}

export function verifyMcpBinary(bytes, platform, architecture) {
  assert.ok(["amd64", "arm64"].includes(architecture), `Unsupported architecture: ${architecture}`);
  assert.ok(bytes.length >= 64, "MCP binary is truncated");
  if (platform === "windows") {
    assert.equal(bytes.toString("ascii", 0, 2), "MZ", "Expected Windows PE binary");
    const offset = bytes.readUInt32LE(0x3c);
    assert.ok(offset + 6 <= bytes.length, "Invalid PE header offset");
    assert.equal(bytes.toString("ascii", offset, offset + 4), "PE\0\0", "Invalid PE header");
    assert.equal(bytes.readUInt16LE(offset + 4), architecture === "amd64" ? 0x8664 : 0xaa64,
      `Wrong Windows MCP architecture: ${architecture}`);
  } else {
    assert.equal(platform, "macos");
    assert.equal(bytes.readUInt32LE(0), 0xfeedfacf, "Expected 64-bit Mach-O binary");
    assert.equal(bytes.readUInt32LE(4), architecture === "amd64" ? 0x01000007 : 0x0100000c,
      `Wrong macOS MCP architecture: ${architecture}`);
  }
}

export async function verifyMcpResources(directory, platform, requireExecutable = platform === "macos") {
  const names = mcpResourceNames(platform);
  assert.deepEqual((await readdir(directory)).sort(), names, "MCP resources are missing or contain another platform");
  const binaries = new Map();
  for (const [index, name] of names.entries()) {
    const file = path.join(directory, name);
    const metadata = await stat(file);
    assert.ok(metadata.isFile(), `${name} must be a file`);
    if (requireExecutable) assert.ok(metadata.mode & 0o111, `${name} is not executable`);
    const bytes = await readFile(file);
    verifyMcpBinary(bytes, platform, index === 0 ? "amd64" : "arm64");
    binaries.set(name, bytes);
  }
  return binaries;
}

export async function verifyMacAppResources(app, expected) {
  const contents = path.join(app, "Contents");
  const resources = path.join(contents, "Resources");
  const relativeNames = (await readdir(resources, { recursive: true }))
    .filter((name) => path.basename(name).startsWith("image-mcp-"))
    .map((name) => name.split(path.sep).join("/"))
    .sort();
  assert.deepEqual(relativeNames, mcpResourceNames("macos").map((name) => `resources/mcp/${name}`),
    "macOS app must contain only its two MCP resources at the runtime lookup path");
  const actual = await verifyMcpResources(path.join(resources, "resources", "mcp"), "macos");
  for (const [name, bytes] of actual) {
    assert.ok(bytes.equals(expected.get(name)), `${name} differs from the generated MCP`);
  }
  const executable = path.join(contents, "MacOS", "711EV-Codex-Tool");
  assert.ok((await stat(executable)).mode & 0o111, "macOS application is not executable");
  return executable;
}
