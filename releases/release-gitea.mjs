import { readFile, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const releasesRoot = path.join(projectRoot, "releases");
const packageJson = JSON.parse(await readFile(path.join(projectRoot, "package.json"), "utf8"));
const version = packageJson.version;
const options = parseArguments(process.argv.slice(2));
const baseUrl = (process.env.GITEA_BASE_URL || "https://git.711ev.com").replace(/\/$/, "");
const owner = process.env.GITEA_OWNER || "711ev";
const repository = process.env.GITEA_REPOSITORY || "711EV-Codex-Tool";
const packageName = process.env.GITEA_PACKAGE || "711ev-codex-tool";
const username = process.env.GITEA_USERNAME || owner;
const token = process.env.GITEA_TOKEN;
const releaseTag = `v${version}`;

if (!token) {
  throw new Error("缺少 GITEA_TOKEN；请先在当前终端设置具有仓库和软件包写入权限的 Gitea 令牌");
}

const packageArtifacts = [
  ["711EV-Codex-Tool.exe", "application/vnd.microsoft.portable-executable"],
  ["711EV-Codex-Tool-Setup.exe", "application/vnd.microsoft.portable-executable"],
  ["711EV-Codex-Tool-Setup.exe.sig", "text/plain; charset=utf-8"],
];
const releaseArtifacts = packageArtifacts.slice(0, 2);
for (const [name] of packageArtifacts) {
  const details = await stat(path.join(releasesRoot, name)).catch(() => null);
  if (!details?.isFile() || details.size === 0) {
    throw new Error(`发布产物不存在或为空：releases/${name}`);
  }
}

const manifestPath = path.join(releasesRoot, "latest.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
if (manifest.version !== version) {
  throw new Error(`更新清单版本 ${manifest.version} 与 package.json ${version} 不一致`);
}
if (options.notes) manifest.notes = options.notes;
manifest.pub_date = new Date().toISOString();
const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");

const existingPackage = await packageVersionExists(version);
const existingRelease = await findReleaseByTag(releaseTag);
if (!options.replace && (existingPackage || existingRelease)) {
  const locations = [
    existingPackage ? `软件包版本 ${version}` : null,
    existingRelease ? `Release ${releaseTag}` : null,
  ].filter(Boolean);
  throw new Error(`${locations.join(" 和 ")} 已存在；确认覆盖时增加 --replace`);
}

if (options.replace) {
  await deletePackageVersion(version);
}

for (const [name, contentType] of packageArtifacts) {
  await upload(version, name, await readFile(path.join(releasesRoot, name)), contentType);
}
await upload(version, "latest.json", manifestBytes, "application/json; charset=utf-8");

// The stable version is intentionally replaced only after every immutable
// version artifact has uploaded successfully.
await deletePackageVersion("latest");
await upload("latest", "latest.json", manifestBytes, "application/json; charset=utf-8");
await publishRelease(existingRelease);

console.log(`已发布 Gitea 软件包 ${packageName} ${version}`);
console.log(`${baseUrl}/${owner}/-/packages/generic/${packageName}/${version}`);
console.log(`已发布 Gitea Release ${releaseTag}`);
console.log(`${baseUrl}/${owner}/${repository}/releases/tag/${releaseTag}`);

async function publishRelease(existingRelease) {
  const releaseData = {
    tag_name: releaseTag,
    target_commitish: process.env.GITEA_TARGET || "master",
    name: `711EV-Codex-Tool ${version}`,
    body: options.notes || `711EV-Codex-Tool ${version}`,
    draft: false,
    prerelease: false,
  };
  const release = existingRelease
    ? await giteaApi(`/repos/${owner}/${repository}/releases/${existingRelease.id}`, {
      method: "PATCH",
      body: releaseData,
    })
    : await giteaApi(`/repos/${owner}/${repository}/releases`, {
      method: "POST",
      body: releaseData,
    });

  const managedNames = new Set(releaseArtifacts.map(([name]) => name));
  for (const asset of release.assets || []) {
    if (managedNames.has(asset.name)) {
      await giteaApi(`/repos/${owner}/${repository}/releases/${release.id}/assets/${asset.id}`, {
        method: "DELETE",
      });
    }
  }

  for (const [name, contentType] of releaseArtifacts) {
    const form = new FormData();
    form.append(
      "attachment",
      new Blob([await readFile(path.join(releasesRoot, name))], { type: contentType }),
      name,
    );
    await giteaApi(
      `/repos/${owner}/${repository}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`,
      { method: "POST", form },
    );
    console.log(`已上传 Release 附件 ${name}`);
  }
}

async function findReleaseByTag(tag) {
  return giteaApi(`/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tag)}`, {
    allowNotFound: true,
  });
}

async function giteaApi(endpoint, options = {}) {
  const headers = { Authorization: `token ${token}` };
  let body;
  if (options.form) {
    body = options.form;
  } else if (options.body !== undefined) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(options.body);
  }
  const response = await fetch(`${baseUrl}/api/v1${endpoint}`, {
    method: options.method || "GET",
    headers,
    body,
  });
  if (options.allowNotFound && response.status === 404) return null;
  if (!response.ok) throw await responseError("Gitea Release 操作失败", response);
  if (response.status === 204) return null;
  return response.json();
}

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
