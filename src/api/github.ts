const repositoryApiUrl = "https://api.github.com/repos/711EV/711EV-Codex-Tool";
const repositoryBadgeApiUrl =
  "https://img.shields.io/github/stars/711EV/711EV-Codex-Tool.json?style=flat&cacheSeconds=300";

export async function fetchRepositoryStarCount(signal?: AbortSignal): Promise<number> {
  try {
    return await fetchGithubStarCount(signal);
  } catch (reason) {
    if (signal?.aborted) throw reason;
    return fetchBadgeStarCount(signal);
  }
}

async function fetchGithubStarCount(signal?: AbortSignal): Promise<number> {
  const payload = await fetchJson(repositoryApiUrl, signal, {
    Accept: "application/vnd.github+json",
  });

  if (
    typeof payload !== "object" ||
    payload === null ||
    !("stargazers_count" in payload) ||
    typeof payload.stargazers_count !== "number"
  ) {
    throw new Error("GitHub API 未返回有效的 Star 数");
  }

  return Math.max(0, Math.trunc(payload.stargazers_count));
}

async function fetchBadgeStarCount(signal?: AbortSignal): Promise<number> {
  const payload = await fetchJson(repositoryBadgeApiUrl, signal);
  if (typeof payload !== "object" || payload === null || !("value" in payload)) {
    throw new Error("Star 备用接口未返回有效数据");
  }

  const value = typeof payload.value === "number" ? String(payload.value) : payload.value;
  if (typeof value !== "string") throw new Error("Star 备用接口返回了无效数值");

  const normalized = value.trim().replaceAll(",", "").toLocaleLowerCase();
  const match = /^(\d+(?:\.\d+)?)([km]?)$/.exec(normalized);
  if (!match) throw new Error("Star 备用接口返回了无法识别的数值");
  const multiplier = match[2] === "k" ? 1_000 : match[2] === "m" ? 1_000_000 : 1;
  return Math.max(0, Math.round(Number(match[1]) * multiplier));
}

async function fetchJson(
  url: string,
  signal?: AbortSignal,
  headers?: Record<string, string>,
): Promise<unknown> {
  const response = await fetch(url, { headers, cache: "no-store", signal });
  if (!response.ok) throw new Error(`Star 接口请求失败：${response.status}`);
  return response.json();
}
