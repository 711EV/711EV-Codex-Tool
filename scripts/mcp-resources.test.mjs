import { afterEach, describe, expect, it } from "vitest";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { mcpResourceNames, verifyMacAppResources, verifyMcpBinary, verifyMcpResources } from "./mcp-resources.mjs";

const temporary = [];
afterEach(async () => {
  await Promise.all(temporary.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

function binary(platform, architecture) {
  const bytes = Buffer.alloc(128);
  if (platform === "windows") {
    bytes.write("MZ");
    bytes.writeUInt32LE(64, 0x3c);
    bytes.write("PE\0\0", 64);
    bytes.writeUInt16LE(architecture === "amd64" ? 0x8664 : 0xaa64, 68);
  } else {
    bytes.writeUInt32LE(0xfeedfacf, 0);
    bytes.writeUInt32LE(architecture === "amd64" ? 0x01000007 : 0x0100000c, 4);
  }
  return bytes;
}

async function resources(platform) {
  const directory = await mkdtemp(path.join(tmpdir(), "711ev-resource-test-"));
  temporary.push(directory);
  const binaries = new Map(mcpResourceNames(platform).map((name, index) => [name, binary(platform, index === 0 ? "amd64" : "arm64")]));
  for (const [name, bytes] of binaries) await writeFile(path.join(directory, name), bytes, { mode: 0o755 });
  return { directory, binaries };
}

describe("MCP release resource validation", () => {
  it.each(["windows", "macos"])("checks both %s architectures and rejects mismatched binaries", (platform) => {
    for (const arch of ["amd64", "arm64"]) {
      expect(() => verifyMcpBinary(binary(platform, arch), platform, arch)).not.toThrow();
      expect(() => verifyMcpBinary(binary(platform, arch), platform, arch === "amd64" ? "arm64" : "amd64")).toThrow();
      expect(() => verifyMcpBinary(binary(platform, arch), platform === "windows" ? "macos" : "windows", arch)).toThrow();
    }
  });

  it("rejects truncated and malformed binary headers", () => {
    expect(() => verifyMcpBinary(Buffer.alloc(3), "windows", "amd64")).toThrow(/truncated/);
    const malformed = binary("windows", "amd64");
    malformed.writeUInt32LE(1024, 0x3c);
    expect(() => verifyMcpBinary(malformed, "windows", "amd64")).toThrow(/offset/);
  });

  it.each(["windows", "macos"])("rejects missing or cross-platform files in %s resources", async (platform) => {
    const { directory, binaries } = await resources(platform);
    expect((await verifyMcpResources(directory, platform, false)).size).toBe(2);
    const foreign = path.join(directory, platform === "macos" ? "image-mcp-windows-amd64.exe" : "image-mcp-darwin-amd64");
    await writeFile(foreign, "foreign");
    await expect(verifyMcpResources(directory, platform, false)).rejects.toThrow(/another platform/);
    await rm(foreign);
    await rm(path.join(directory, [...binaries.keys()][0]));
    await expect(verifyMcpResources(directory, platform, false)).rejects.toThrow(/missing/);
  });

  it.skipIf(process.platform === "win32")("checks app lookup paths, execution permissions and resource contents", async () => {
    const { directory, binaries } = await resources("macos");
    const app = path.join(directory, "Test.app");
    const mcp = path.join(app, "Contents/Resources/resources/mcp");
    await mkdir(mcp, { recursive: true });
    await mkdir(path.join(app, "Contents/MacOS"), { recursive: true });
    await writeFile(path.join(app, "Contents/MacOS/711EV-Codex-Tool"), "fixture", { mode: 0o755 });
    for (const [name, bytes] of binaries) await writeFile(path.join(mcp, name), bytes, { mode: 0o755 });
    await expect(verifyMacAppResources(app, binaries)).resolves.toContain("711EV-Codex-Tool");
    const first = path.join(mcp, [...binaries.keys()][0]);
    await chmod(first, 0o644);
    await expect(verifyMacAppResources(app, binaries)).rejects.toThrow(/not executable/);
    await chmod(first, 0o755);
    const modified = Buffer.from([...binaries.values()][0]);
    modified[100] = 1;
    await writeFile(first, modified);
    await expect(verifyMacAppResources(app, binaries)).rejects.toThrow(/differs/);
    await writeFile(path.join(mcp, "image-mcp-windows-arm64.exe"), "foreign");
    await expect(verifyMacAppResources(app, binaries)).rejects.toThrow(/only its two MCP/);
  });
});
