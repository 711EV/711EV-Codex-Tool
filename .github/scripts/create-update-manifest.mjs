import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const [assetsArgument, repository] = process.argv.slice(2);
if (!assetsArgument || !repository) {
  throw new Error("usage: node create-update-manifest.mjs <assets-dir> <owner/repository>");
}

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const assetsRoot = path.resolve(assetsArgument);
const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
const version = packageJson.version;
const tag = `v${version}`;
const downloadBase = `https://github.com/${repository}/releases/download/${tag}`;
const windowsInstaller = `711EV-Codex-Tool-${version}-Windows-安装包.exe`;
const macUpdater = `711EV-Codex-Tool-${version}-macOS-自动更新包.app.tar.gz`;
const windowsSignature = await signature(`${windowsInstaller}.sig`);
const macSignature = await signature(`${macUpdater}.sig`);
const macPlatform = {
  signature: macSignature,
  url: assetUrl(macUpdater),
};

const manifest = {
  version,
  notes: `711EV-Codex-Tool ${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: windowsSignature,
      url: assetUrl(windowsInstaller),
    },
    "darwin-x86_64": macPlatform,
    "darwin-aarch64": macPlatform,
  },
};

function assetUrl(name) {
  return `${downloadBase}/${encodeURIComponent(name)}`;
}

await writeFile(
  path.join(assetsRoot, "latest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
  "utf8",
);

async function signature(name) {
  const value = (await readFile(path.join(assetsRoot, name), "utf8")).trim();
  if (!value) throw new Error(`empty updater signature: ${name}`);
  return value;
}
