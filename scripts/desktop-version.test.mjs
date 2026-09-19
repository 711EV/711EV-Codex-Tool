import { describe, expect, it } from "vitest";
import { nextDesktopVersion, parseDesktopVersion } from "./desktop-version.mjs";

describe("desktop version increments", () => {
  it.each([
    ["0.0.0", "0.0.1"],
    ["1.2.1", "1.2.2"],
    ["1.2.9", "1.2.10"],
    ["1.1.10", "1.2.1"],
    ["1.2.10", "1.3.1"],
    ["1.0.10", "1.1.1"],
    ["1.9.10", "1.10.1"],
    ["1.10.0", "1.10.1"],
    ["1.10.9", "1.10.10"],
    ["1.10.10", "2.1.1"],
    ["10.10.10", "11.1.1"],
  ])("increments %s to %s", (current, next) => {
    expect(nextDesktopVersion(current)).toBe(next);
  });

  it("accepts every minor and patch value from 0 through 10", () => {
    for (let minor = 0; minor <= 10; minor += 1) {
      for (let patch = 0; patch <= 10; patch += 1) {
        expect(parseDesktopVersion(`1.${minor}.${patch}`)).toEqual({ major: 1, minor, patch });
      }
    }
  });

  it.each([
    "1.11.1", "1.2.11", "1.99.99", "1.1", "1.1.1.1", "-1.1.1",
    "01.1.1", "1.01.1", "1.1.01", "v1.1.1", "1.1.1-beta", "1.1.1+build",
    " 1.1.1", "1.1.1 ", "", null, undefined,
  ])("rejects invalid version %s without computing a build version", (version) => {
    expect(() => parseDesktopVersion(version)).toThrow("minor 和 patch 只能是 0-10");
    expect(() => nextDesktopVersion(version)).toThrow("minor 和 patch 只能是 0-10");
  });
});
