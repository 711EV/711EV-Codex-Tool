export function parseDesktopVersion(version) {
  const value = String(version ?? "");
  const match = /^(0|[1-9]\d*)\.(0|[1-9]|10)\.(0|[1-9]|10)$/.exec(value);

  if (!match) {
    throw new Error(
      `无效的桌面端版本号: ${value}。版本格式必须为 major.minor.patch，minor 和 patch 只能是 0-10。`,
    );
  }

  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
  };
}

export function nextDesktopVersion(version) {
  let { major, minor, patch } = parseDesktopVersion(version);

  if (patch < 10) {
    patch += 1;
  } else {
    patch = 1;
    minor += 1;
  }
  if (minor > 10) {
    minor = 1;
    major += 1;
  }

  return `${major}.${minor}.${patch}`;
}
