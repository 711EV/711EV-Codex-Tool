import { cp, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import path from "node:path";
import process from "node:process";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const targetRoot = path.join(projectRoot, "src-tauri", "target", "release");
const distRoot = path.join(projectRoot, "dist");
const releasesRoot = path.join(projectRoot, "releases");
const portableDataDirName = "CodexLocalSync.data";
const portableExecutableName = "711EV-Codex-Tool.exe";
const installerName = "711EV-Codex-Tool-Setup.exe";
const installerSignatureName = `${installerName}.sig`;
const updatePackageBaseUrl =
  "https://git.711ev.com/api/packages/711ev/generic/711ev-codex-tool";
const platform = process.platform;
const versionFiles = [
  path.join(projectRoot, "package.json"),
  path.join(projectRoot, "package-lock.json"),
  path.join(projectRoot, "src-tauri", "tauri.conf.json"),
  path.join(projectRoot, "src-tauri", "Cargo.toml"),
  path.join(projectRoot, "src-tauri", "Cargo.lock"),
];

if (platform !== "win32" && platform !== "darwin") {
  throw new Error("桌面构建目前只支持 Windows 和 macOS");
}

await runNode([path.join("node_modules", "vue-tsc", "bin", "vue-tsc.js"), "--noEmit"]);
const originalVersionFiles = new Map(
  await Promise.all(
    versionFiles.map(async (filePath) => [filePath, await readFile(filePath, "utf8")]),
  ),
);
const currentPackage = JSON.parse(originalVersionFiles.get(versionFiles[0]));
const buildVersion = incrementPatchVersion(currentPackage.version);

if (!process.env.TAURI_SIGNING_PRIVATE_KEY && !process.env.TAURI_SIGNING_PRIVATE_KEY_PATH) {
  const defaultSigningKey = path.join(homedir(), ".tauri", "711ev-codex-tool.key");
  if (await pathExists(defaultSigningKey)) {
    process.env.TAURI_SIGNING_PRIVATE_KEY_PATH = defaultSigningKey;
  }
}
if (!process.env.TAURI_SIGNING_PRIVATE_KEY && process.env.TAURI_SIGNING_PRIVATE_KEY_PATH) {
  process.env.TAURI_SIGNING_PRIVATE_KEY = (
    await readFile(process.env.TAURI_SIGNING_PRIVATE_KEY_PATH, "utf8")
  ).trim();
}
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  throw new Error(
    "缺少升级签名私钥；请设置 TAURI_SIGNING_PRIVATE_KEY_PATH 或生成 ~/.tauri/711ev-codex-tool.key",
  );
}
if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) {
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "";
}

try {
  await writeBuildVersion(buildVersion);
  await runNode([path.join("node_modules", "vite", "bin", "vite.js"), "build"]);

  const system = platform === "win32" ? "windows" : "macos";

  if (platform === "win32") {
    await runNode([
      path.join("node_modules", "@tauri-apps", "cli", "tauri.js"),
      "build",
      "--bundles",
      "nsis",
    ]);
    await prepareOutputDirectories(system);
    await Promise.all([
      cp(
        path.join(targetRoot, portableExecutableName),
        path.join(distRoot, portableExecutableName),
      ),
      cp(
        path.join(targetRoot, portableExecutableName),
        path.join(releasesRoot, portableExecutableName),
      ),
    ]);
    await copyWindowsInstallerArtifacts(buildVersion);
  } else {
    await runNode([
      path.join("node_modules", "@tauri-apps", "cli", "tauri.js"),
      "build",
      "--bundles",
      "app",
    ]);
    await prepareOutputDirectories(system);
    const outputApp = path.join(distRoot, "711EV-Codex-Tool.app");
    await cp(path.join(targetRoot, "bundle", "macos", "711EV-Codex-Tool.app"), outputApp, {
      recursive: true,
    });
  }

  console.log(`版本：v${buildVersion}`);
  console.log(`Vue 文件：${path.join(projectRoot, "build")}`);
  console.log(`预览程序：${distRoot}`);
  console.log(`发布产物：${releasesRoot}`);
} catch (error) {
  await Promise.all(
    [...originalVersionFiles].map(([filePath, contents]) => writeFile(filePath, contents, "utf8")),
  );
  throw error;
}

async function copyWindowsInstallerArtifacts(version) {
  const bundleDir = path.join(targetRoot, "bundle", "nsis");
  const files = await readdir(bundleDir);
  const installerSource = singleFile(
    files,
    (name) => name.includes(`_${version}_`) && name.endsWith("-setup.exe"),
    "NSIS 安装程序",
  );
  const signatureSource = `${installerSource}.sig`;
  if (!files.includes(signatureSource)) {
    throw new Error(`缺少升级包签名：${signatureSource}`);
  }

  await Promise.all([
    cp(path.join(bundleDir, installerSource), path.join(releasesRoot, installerName)),
    cp(path.join(bundleDir, signatureSource), path.join(releasesRoot, installerSignatureName)),
  ]);

  const signature = (await readFile(path.join(bundleDir, signatureSource), "utf8")).trim();
  const platformKey = `windows-${process.arch === "arm64" ? "aarch64" : "x86_64"}`;
  await writeJson(path.join(releasesRoot, "latest.json"), {
    version,
    notes: `711EV-Codex-Tool ${version}`,
    pub_date: new Date().toISOString(),
    platforms: {
      [platformKey]: {
        signature,
        url: `${updatePackageBaseUrl}/${version}/${installerName}`,
      },
    },
  });
}

function singleFile(files, predicate, label) {
  const matches = files.filter(predicate);
  if (matches.length !== 1) {
    throw new Error(`${label}数量不正确：${matches.join(", ") || "未找到"}`);
  }
  return matches[0];
}

async function prepareOutputDirectories(system) {
  assertManagedDirectory(distRoot, "dist");
  assertManagedDirectory(releasesRoot, "releases");
  await Promise.all([
    mkdir(distRoot, { recursive: true }),
    mkdir(releasesRoot, { recursive: true }),
  ]);
  await migrateLegacyPortableData(distRoot, system);

  const entries = await readdir(distRoot, { withFileTypes: true });
  for (const entry of entries) {
    if (entry.name === portableDataDirName) continue;
    await rm(path.join(distRoot, entry.name), { recursive: true, force: true });
  }

  const releaseScripts = new Set(["release-gitea.mjs"]);
  const releaseEntries = await readdir(releasesRoot, { withFileTypes: true });
  for (const entry of releaseEntries) {
    if (releaseScripts.has(entry.name)) continue;
    await rm(path.join(releasesRoot, entry.name), { recursive: true, force: true });
  }
}

function assertManagedDirectory(directory, expectedName) {
  const resolved = path.resolve(directory);
  if (path.dirname(resolved) !== projectRoot || path.basename(resolved) !== expectedName) {
    throw new Error(`拒绝清理非项目 ${expectedName} 目录：${resolved}`);
  }
}

async function migrateLegacyPortableData(outputDir, system) {
  const currentDataDir = path.join(outputDir, portableDataDirName);
  if (await pathExists(currentDataDir)) return;

  const versionDirectories = (await readdir(outputDir, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory() && /^\d+\.\d+\.\d+$/.test(entry.name))
    .sort((left, right) => compareVersions(right.name, left.name));

  for (const versionDirectory of versionDirectories) {
    const legacyDataDir = path.join(
      outputDir,
      versionDirectory.name,
      system,
      portableDataDirName,
    );
    if (!(await pathExists(legacyDataDir))) continue;

    await cp(legacyDataDir, currentDataDir, { recursive: true });
    console.log(`已迁移运行数据：${legacyDataDir} -> ${currentDataDir}`);
    return;
  }
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

async function pathExists(filePath) {
  try {
    await stat(filePath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function incrementPatchVersion(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!match) throw new Error(`不支持的版本格式：${version}`);
  return `${match[1]}.${match[2]}.${Number(match[3]) + 1}`;
}

async function writeBuildVersion(version) {
  const packagePath = versionFiles[0];
  const packageLockPath = versionFiles[1];
  const tauriConfigPath = versionFiles[2];
  const cargoManifestPath = versionFiles[3];
  const packageJson = JSON.parse(await readFile(packagePath, "utf8"));
  const packageLock = JSON.parse(await readFile(packageLockPath, "utf8"));
  const tauriConfig = JSON.parse(await readFile(tauriConfigPath, "utf8"));
  const cargoManifest = await readFile(cargoManifestPath, "utf8");

  packageJson.version = version;
  packageLock.version = version;
  if (packageLock.packages?.[""]) packageLock.packages[""].version = version;
  tauriConfig.version = version;
  const updatedCargoManifest = cargoManifest.replace(
    /(\[package\][\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m,
    `$1${version}$2`,
  );
  if (updatedCargoManifest === cargoManifest) {
    throw new Error("未能更新 src-tauri/Cargo.toml 版本号");
  }

  await Promise.all([
    writeJson(packagePath, packageJson),
    writeJson(packageLockPath, packageLock),
    writeJson(tauriConfigPath, tauriConfig),
    writeFile(cargoManifestPath, updatedCargoManifest, "utf8"),
  ]);
}

function writeJson(filePath, value) {
  return writeFile(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function runNode(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, {
      cwd: projectRoot,
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`构建失败，退出码：${code ?? "unknown"}`));
    });
  });
}
