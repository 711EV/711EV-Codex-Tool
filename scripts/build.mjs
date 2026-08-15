import { cp, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const targetRoot = path.join(projectRoot, "src-tauri", "target", "release");
const distRoot = path.join(projectRoot, "dist");
const portableDataDirName = "CodexLocalSync.data";
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

try {
  await writeBuildVersion(buildVersion);
  await runNode([path.join("node_modules", "vite", "bin", "vite.js"), "build"]);

  const system = platform === "win32" ? "windows" : "macos";

  if (platform === "win32") {
    await runNode([
      path.join("node_modules", "@tauri-apps", "cli", "tauri.js"),
      "build",
      "--no-bundle",
    ]);
    await prepareOutputDirectory(system);
    await cp(
      path.join(targetRoot, "codex-local-sync.exe"),
      path.join(distRoot, "711EV-Codex-Tool.exe"),
    );
  } else {
    await runNode([
      path.join("node_modules", "@tauri-apps", "cli", "tauri.js"),
      "build",
      "--bundles",
      "app",
    ]);
    await prepareOutputDirectory(system);
    const outputApp = path.join(distRoot, "711EV-Codex-Tool.app");
    await cp(path.join(targetRoot, "bundle", "macos", "711EV-Codex-Tool.app"), outputApp, {
      recursive: true,
    });
  }

  console.log(`版本：v${buildVersion}`);
  console.log(`Vue 文件：${path.join(projectRoot, "build")}`);
  console.log(`客户端文件：${distRoot}`);
} catch (error) {
  await Promise.all(
    [...originalVersionFiles].map(([filePath, contents]) => writeFile(filePath, contents, "utf8")),
  );
  throw error;
}

async function prepareOutputDirectory(system) {
  const resolvedDistRoot = path.resolve(distRoot);
  if (
    path.dirname(resolvedDistRoot) !== projectRoot ||
    path.basename(resolvedDistRoot) !== "dist"
  ) {
    throw new Error(`拒绝清理非项目 dist 目录：${resolvedDistRoot}`);
  }

  await mkdir(resolvedDistRoot, { recursive: true });
  await migrateLegacyPortableData(resolvedDistRoot, system);

  const entries = await readdir(resolvedDistRoot, { withFileTypes: true });
  for (const entry of entries) {
    if (entry.name === portableDataDirName) continue;
    await rm(path.join(resolvedDistRoot, entry.name), { recursive: true, force: true });
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
