import { readFile, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = path.join(projectRoot, "dist");
const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
const version = packageJson.version;
const options = parseArguments(process.argv.slice(2));
const baseUrl = (process.env.GITEA_BASE_URL || "https://git.711ev.com").replace(/\/$/, "");
const owner = process.env.GITEA_OWNER || "711ev";
const packageName = process.env.GITEA_PACKAGE || "711ev-codex-tool";
const username = process.env.GITEA_USERNAME || owner;
const token = process.env.GITEA_TOKEN;

if (!token) {
  throw new Error("缺少 GITEA_TOKEN；请先在当前终端设置 Gitea 软件包写入令牌");
}

const artifacts = [
  ["711EV-Codex-Tool.exe", "application/vnd.microsoft.portable-executable"],
  ["711EV-Codex-Tool-Setup.exe", "application/vnd.microsoft.portable-executable"],
  ["711EV-Codex-Tool-Setup.exe.sig", "text/plain; charset=utf-8"],
];
for (const [name] of artifacts) {
  const details = await stat(path.join(distRoot, name)).catch(() => null);
  if (!details?.isFile() || details.size === 0) {
    throw new Error(`发布产物不存在或为空：dist/${name}`);
  }
}

const manifestPath = path.join(distRoot, "latest.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
if (manifest.version !== version) {
  throw new Error(`更新清单版本 ${manifest.version} 与 package.json ${version} 不一致`);
}
if (options.notes) manifest.notes = options.notes;
manifest.pub_date = new Date().toISOString();
const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");

if (options.replace) {
  await deletePackageVersion(version);
} else if (await packageVersionExists(version)) {
  throw new Error(`软件包版本 ${version} 已存在；确认覆盖时增加 --replace`);
}

for (const [name, contentType] of artifacts) {
  await upload(version, name, await readFile(path.join(distRoot, name)), contentType);
}
await upload(version, "latest.json", manifestBytes, "application/json; charset=utf-8");

// The stable version is intentionally replaced only after every immutable
// version artifact has uploaded successfully.
await deletePackageVersion("latest");
await upload("latest", "latest.json", manifestBytes, "application/json; charset=utf-8");

console.log(`已发布 Gitea 软件包 ${packageName} ${version}`);
console.log(`${baseUrl}/${owner}/-/packages/generic/${packageName}/${version}`);

async function packageVersionExists(packageVersion) {
  const response = await fetch(apiPackageVersionUrl(packageVersion), {
    headers: apiHeaders(),
  });
  if (response.status === 404) return false;
  if (!response.ok) throw await responseError("检查软件包版本失败", response);
  return true;
}

async function deletePackageVersion(packageVersion) {
  const response = await fetch(apiPackageVersionUrl(packageVersion), {
    method: "DELETE",
    headers: apiHeaders(),
  });
  if (response.status === 404) return;
  if (!response.ok) throw await responseError(`删除软件包版本 ${packageVersion} 失败`, response);
}

async function upload(packageVersion, fileName, bytes, contentType) {
  const url = [baseUrl, "api", "packages", owner, "generic", packageName, packageVersion, fileName]
    .map((part, index) => index === 0 ? part : encodeURIComponent(part))
    .join("/");
  const response = await fetch(url, {
    method: "PUT",
    headers: {
      Authorization: `Basic ${Buffer.from(`${username}:${token}`).toString("base64")}`,
      "Content-Type": contentType,
      "Content-Length": String(bytes.length),
    },
    body: bytes,
  });
  if (!response.ok) throw await responseError(`上传 ${fileName} 失败`, response);
  console.log(`已上传 ${packageVersion}/${fileName}`);
}

function apiPackageVersionUrl(packageVersion) {
  return `${baseUrl}/api/v1/packages/${encodeURIComponent(owner)}/generic/${encodeURIComponent(packageName)}/${encodeURIComponent(packageVersion)}`;
}

function apiHeaders() {
  return { Authorization: `token ${token}` };
}

async function responseError(prefix, response) {
  const body = (await response.text()).trim();
  return new Error(`${prefix}：HTTP ${response.status}${body ? ` ${body}` : ""}`);
}

function parseArguments(args) {
  let notes = "";
  let replace = false;
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--replace") {
      replace = true;
    } else if (args[index] === "--notes") {
      notes = args[index + 1] || "";
      index += 1;
    } else {
      throw new Error(`不支持的发布参数：${args[index]}`);
    }
  }
  return { notes, replace };
}
