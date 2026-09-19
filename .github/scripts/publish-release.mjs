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
const releaseHighlightsByVersion = new Map([
  [
    "1.2.3",
    `- 新增生图功能。
- 新增环境检测。
- 优化用户体验。`,
  ],
  [
    "1.1.8",
    `- 产品界面、窗口标题、安装快捷方式与应用图标统一升级为“ChatGPT中转工具”，保留原可执行文件名和升级标识以兼容已有安装。
- 重构 ChatGPT Desktop 探测与重启：支持 Windows Store、独立安装和 macOS App，按目标 \`CODEX_HOME\` 安全匹配主进程并排除 CLI、App Server 与 Chromium 辅助进程。
- 全新竖屏供应商工作台：支持多配置目录、供应商状态与会话统计、711EV 配置预填、单供应商刷新，以及 API 地址/API 密钥查看与复制。
- 完善官方账号与中转供应商切换：官方账号隐私信息不展示，中转配置使用事务化原子写入，并保留官方 OAuth 本地快照恢复能力。
- 重做会话恢复与清理：只恢复其他供应商的主会话，清理严格限定当前打开的供应商，支持多选删除归档会话和子会话，并在操作前友好确认关闭 ChatGPT。
- 对齐 desk 暗色桌面体验：系统托盘、公共弹窗、Message 队列、Tooltip、DPI 自适应、8px 圆角及四周暗色阴影；移除前端迁移入口。`,
  ],
  [
    "0.1.102",
    `- 收紧 Codex 存储位置识别规则，不再把仅包含 sessions 等通用目录结构的 Claude 或普通目录识别为 Codex 存储位置。
- 重新发现时校验本地索引：明确无效的记录只从工具索引中清理，暂时不存在或无法访问的位置会保留为暂不可用，不删除用户目录或会话文件。
- 复制和迁移会话增加逐条进度、当前处理阶段以及成功、跳过、失败状态。
- 顶部资源区增加使用教程入口，并调整 QQ 交流群入口的图标与位置。`,
  ],
]);
if (tag !== `v${version}`) {
  throw new Error(`tag ${tag} does not match package version ${version}`);
}
const releaseHighlights = releaseHighlightsByVersion.get(version) ?? "本版本包含稳定性和兼容性改进。";

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
const notes = `## 本次更新

${releaseHighlights}

## 下载说明

本版本提供以下下载文件：

- Windows 便携版：无需安装，下载后可直接运行。
- Windows 安装包：支持选择安装目录，安装后创建“ChatGPT中转工具”桌面快捷方式。
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
    `ChatGPT中转工具 ${tag}`,
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
    `ChatGPT中转工具 ${tag}`,
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
