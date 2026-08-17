import { readdir, readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const [assetsArgument, repository, tag] = process.argv.slice(2);
if (!assetsArgument || !repository || !tag) {
  throw new Error(
    "usage: node publish-release.mjs <assets-dir> <owner/repository> <tag>",
  );
}
if (!process.env.GH_TOKEN) {
  throw new Error("GH_TOKEN is required to publish a release");
}

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const assetsRoot = path.resolve(assetsArgument);
const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
const version = packageJson.version;
if (tag !== `v${version}`) {
  throw new Error(`tag ${tag} does not match package version ${version}`);
}

const labels = new Map([
  [`711EV-Codex-Tool-${version}-Windows-portable.exe`, `Windows 便携版（版本 ${version}，Windows x64）`],
  [`711EV-Codex-Tool-${version}-Windows-setup.exe`, `Windows 安装包（版本 ${version}，Windows x64）`],
  [`711EV-Codex-Tool-${version}-Windows-setup.exe.sig`, `Windows 安装包签名（版本 ${version}）`],
  [`711EV-Codex-Tool-${version}-macOS-universal.dmg`, `macOS 通用安装包（版本 ${version}，Intel / Apple Silicon）`],
  [`711EV-Codex-Tool-${version}-macOS-updater.app.tar.gz`, `macOS 自动更新包（版本 ${version}，Intel / Apple Silicon）`],
  [`711EV-Codex-Tool-${version}-macOS-updater.app.tar.gz.sig`, `macOS 自动更新签名（版本 ${version}）`],
  ["latest.json", `自动更新清单（版本 ${version}）`],
]);
const files = (await readdir(assetsRoot, { withFileTypes: true }))
  .filter((entry) => entry.isFile())
  .map((entry) => entry.name)
  .sort();
const expected = [...labels.keys()].sort();
if (JSON.stringify(files) !== JSON.stringify(expected)) {
  throw new Error(
    `release assets do not match the expected set\nexpected: ${expected.join(", ")}\nactual: ${files.join(", ")}`,
  );
}
const assetPaths = files.map((name) => path.join(assetsRoot, name));
const notes = `本版本提供以下下载文件：

- Windows 便携版：无需安装，下载后可直接运行。
- Windows 安装包：支持选择安装目录并创建桌面快捷方式。
- macOS 通用安装包：同时支持 Intel 与 Apple Silicon 设备。

\`.sig\` 和 \`latest.json\` 为应用自动更新所需文件，普通用户无需手动下载。`;
const releaseExists = runGh(["release", "view", tag, "--repo", repository], true).ok;
if (releaseExists) {
  runGh(["release", "upload", tag, "--repo", repository, "--clobber", ...assetPaths]);
  runGh([
    "release",
    "edit",
    tag,
    "--repo",
    repository,
    "--title",
    `711EV-Codex-Tool ${tag}`,
    "--notes",
    notes,
    "--draft=false",
  ]);
} else {
  runGh([
    "release",
    "create",
    tag,
    "--repo",
    repository,
    "--verify-tag",
    "--title",
    `711EV-Codex-Tool ${tag}`,
    "--notes",
    notes,
    ...assetPaths,
  ]);
}

const release = JSON.parse(
  runGh(["api", `repos/${repository}/releases/tags/${tag}`]).stdout,
);
const assets = JSON.parse(
  runGh(["api", `repos/${repository}/releases/${release.id}/assets?per_page=100`]).stdout,
);
const uploaded = new Set();
for (const asset of assets) {
  const label = labels.get(asset.name);
  if (!label) {
    runGh(["api", "--method", "DELETE", `repos/${repository}/releases/assets/${asset.id}`]);
    continue;
  }
  uploaded.add(asset.name);
  runGh([
    "api",
    "--method",
    "PATCH",
    `repos/${repository}/releases/assets/${asset.id}`,
    "-f",
    `name=${asset.name}`,
    "-f",
    `label=${label}`,
  ]);
}
const missing = expected.filter((name) => !uploaded.has(name));
if (missing.length > 0) {
  throw new Error(`missing release assets after upload: ${missing.join(", ")}`);
}

function runGh(arguments_, allowFailure = false) {
  const result = spawnSync("gh", arguments_, {
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) throw result.error;
  const ok = result.status === 0;
  if (!ok && !allowFailure) {
    const detail = result.stderr.trim() || result.stdout.trim() || `exit code ${result.status}`;
    throw new Error(`gh ${arguments_[0]} failed: ${detail}`);
  }
  return { ok, stdout: result.stdout, stderr: result.stderr };
}
