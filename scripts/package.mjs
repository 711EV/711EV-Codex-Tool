import { execFile, spawn } from "node:child_process";
import { readFile, readdir } from "node:fs/promises";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const execFileAsync = promisify(execFile);
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = path.join(projectRoot, "dist");
const isWindows = process.platform === "win32";
const isMac = process.platform === "darwin";
const artifact = path.join(
  distRoot,
  isWindows ? "711EV-Codex-Tool.exe" : "711EV-Codex-Tool.app",
);

if (!isWindows && !isMac) {
  throw new Error("桌面打包目前只支持 Windows 和 macOS");
}

let stoppedCount = 0;
try {
  stoppedCount = await stopRunningArtifact();
  await run(process.execPath, [path.join("node_modules", "vitest", "vitest.mjs"), "run"]);
  await run("cargo", ["fmt", "--manifest-path", "src-tauri/Cargo.toml", "--", "--check"]);
  await run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml"]);
  await run(process.execPath, [path.join("scripts", "build.mjs")]);
  await launchArtifact();
  const runtime = await waitForRunningArtifact();
  await verifyDistContents();

  const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
  console.log("");
  console.log("桌面打包完成");
  console.log(`版本号：${packageJson.version}`);
  console.log(`程序：${artifact}`);
  console.log(`进程：${runtime}`);
  console.log("dist：仅保留客户端和 CodexLocalSync.data");
} catch (error) {
  if (stoppedCount > 0) {
    await launchArtifact().catch(() => {});
  }
  throw error;
}

async function stopRunningArtifact() {
  if (isWindows) {
    const script = String.raw`
$target = [System.IO.Path]::GetFullPath($env:CODEX_SYNC_PACKAGE_EXE)
$owned = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -eq '711EV-Codex-Tool.exe' -and $_.ExecutablePath -eq $target
}
foreach ($item in $owned) {
  $process = Get-Process -Id $item.ProcessId -ErrorAction SilentlyContinue
  if ($process -and $process.CloseMainWindow()) {
    $null = $process.WaitForExit(3000)
  }
  if (Get-Process -Id $item.ProcessId -ErrorAction SilentlyContinue) {
    Stop-Process -Id $item.ProcessId -Force
  }
}
Start-Sleep -Milliseconds 250
$remaining = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -eq '711EV-Codex-Tool.exe' -and $_.ExecutablePath -eq $target
}
if ($remaining) { throw '711EV-Codex-Tool 仍在运行' }
Write-Output @($owned).Count
`;
    const { stdout } = await runPowerShell(script);
    return Number(stdout.trim()) || 0;
  }

  const processes = await findMacProcesses();
  for (const pid of processes) process.kill(pid, "SIGTERM");
  await wait(500);
  const remaining = await findMacProcesses();
  for (const pid of remaining) process.kill(pid, "SIGKILL");
  return processes.length;
}

async function launchArtifact() {
  if (isWindows) {
    const child = spawn(artifact, [], {
      cwd: distRoot,
      detached: true,
      stdio: "ignore",
      windowsHide: false,
    });
    child.unref();
    return;
  }
  await execFileAsync("open", [artifact], { cwd: projectRoot });
}

async function waitForRunningArtifact() {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (isWindows) {
      const status = await windowsArtifactStatus();
      if (status?.responding) return `PID ${status.pid}，响应正常`;
    } else {
      const processes = await findMacProcesses();
      if (processes.length > 0) return `PID ${processes[0]}，已启动`;
    }
    await wait(500);
  }
  throw new Error("客户端启动后未进入正常响应状态");
}

async function windowsArtifactStatus() {
  const script = String.raw`
$target = [System.IO.Path]::GetFullPath($env:CODEX_SYNC_PACKAGE_EXE)
$item = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -eq '711EV-Codex-Tool.exe' -and $_.ExecutablePath -eq $target
} | Select-Object -First 1
if (-not $item) { exit 3 }
$process = Get-Process -Id $item.ProcessId -ErrorAction Stop
[PSCustomObject]@{
  pid = $process.Id
  responding = $process.Responding
} | ConvertTo-Json -Compress
`;
  try {
    const { stdout } = await runPowerShell(script);
    return JSON.parse(stdout.trim());
  } catch {
    return null;
  }
}

async function findMacProcesses() {
  const executable = path.join(artifact, "Contents", "MacOS", "711EV-Codex-Tool");
  let stdout;
  try {
    ({ stdout } = await execFileAsync("ps", ["-axo", "pid=,command="], {
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
    }));
  } catch {
    return [];
  }
  return stdout
    .split(/\r?\n/)
    .map((line) => /^(\s*\d+)\s+(.+)$/.exec(line))
    .filter((match) => match && (match[2] === executable || match[2].startsWith(`${executable} `)))
    .map((match) => Number(match[1]));
}

async function verifyDistContents() {
  const expected = new Set([path.basename(artifact), "CodexLocalSync.data"]);
  const actual = await readdir(distRoot);
  const missing = [...expected].filter((name) => !actual.includes(name));
  const unexpected = actual.filter((name) => !expected.has(name));
  if (missing.length || unexpected.length) {
    throw new Error(
      `dist 结构不正确；缺少：${missing.join(", ") || "无"}；多余：${unexpected.join(", ") || "无"}`,
    );
  }
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: projectRoot,
      stdio: "inherit",
      shell: false,
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} 执行失败，退出码：${code ?? "unknown"}`));
    });
  });
}

function runPowerShell(script) {
  return execFileAsync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
    {
      cwd: projectRoot,
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
      env: {
        ...process.env,
        CODEX_SYNC_PACKAGE_EXE: artifact,
      },
    },
  );
}

function wait(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
