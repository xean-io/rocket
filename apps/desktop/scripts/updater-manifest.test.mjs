// @vitest-environment node
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { DEFAULT_NOTES, buildManifest, main, normalizeVersion } from "./updater-manifest.mjs";

const base = {
  version: "v0.2.0",
  notes: "Fixes and polish.",
  pubDate: new Date("2026-10-06T12:00:00.123Z"),
  url: "https://github.com/xean-io/rocket/releases/download/v0.2.0/Rocket.app.tar.gz",
  signature: "  c2lnbmF0dXJl\n",
};

describe("normalizeVersion", () => {
  it("drops one leading v and rejects non-semver input", () => {
    expect(normalizeVersion("v0.2.0")).toBe("0.2.0");
    expect(normalizeVersion("0.2.0-rc.1")).toBe("0.2.0-rc.1");
    expect(() => normalizeVersion("latest")).toThrow(/version/);
    expect(() => normalizeVersion("")).toThrow(/version/);
  });
});

describe("buildManifest", () => {
  it("emits the static updater format for both macOS architectures", () => {
    const m = buildManifest(base);
    expect(m.version).toBe("0.2.0");
    expect(m.notes).toBe("Fixes and polish.");
    expect(m.pub_date).toBe("2026-10-06T12:00:00Z");
    const platform = { url: base.url, signature: "c2lnbmF0dXJl" };
    expect(m.platforms).toEqual({
      "darwin-aarch64": platform,
      "darwin-x86_64": platform,
    });
  });

  it("falls back to a short default when the release has no notes", () => {
    expect(buildManifest({ ...base, notes: "  \n" }).notes).toBe(DEFAULT_NOTES);
    expect(buildManifest({ ...base, notes: undefined }).notes).toBe(DEFAULT_NOTES);
  });

  it("caps very long release notes", () => {
    const notes = buildManifest({ ...base, notes: "x".repeat(10_000) }).notes;
    expect(notes.length).toBeLessThanOrEqual(2000);
    expect(notes.endsWith("…")).toBe(true);
  });

  it("refuses an empty signature or a missing url", () => {
    expect(() => buildManifest({ ...base, signature: " \n" })).toThrow(/signature/);
    expect(() => buildManifest({ ...base, url: "" })).toThrow(/url/);
  });
});

describe("main", () => {
  it("writes latest.json from files and flags", () => {
    const dir = mkdtempSync(join(tmpdir(), "rkt-manifest-"));
    writeFileSync(join(dir, "app.sig"), "c2lnbmF0dXJl\n");
    writeFileSync(join(dir, "notes.md"), "## What's new\n- things\n");
    const out = join(dir, "latest.json");
    main([
      "--version", "v0.2.0",
      "--url", base.url,
      "--sig-file", join(dir, "app.sig"),
      "--notes-file", join(dir, "notes.md"),
      "--pub-date", "2026-10-06T12:00:00Z",
      "--out", out,
    ]);
    const written = JSON.parse(readFileSync(out, "utf8"));
    expect(written.version).toBe("0.2.0");
    expect(written.notes).toBe("## What's new\n- things");
    expect(written.platforms["darwin-aarch64"].signature).toBe("c2lnbmF0dXJl");
  });

  it("tolerates a missing notes file", () => {
    const dir = mkdtempSync(join(tmpdir(), "rkt-manifest-"));
    writeFileSync(join(dir, "app.sig"), "c2ln");
    const out = join(dir, "latest.json");
    main(["--version", "0.2.0", "--url", base.url, "--sig-file", join(dir, "app.sig"), "--notes-file", join(dir, "none.md"), "--out", out]);
    expect(JSON.parse(readFileSync(out, "utf8")).notes).toBe(DEFAULT_NOTES);
  });

  it("rejects missing required flags", () => {
    expect(() => main(["--version", "0.2.0"])).toThrow(/--url/);
  });
});
