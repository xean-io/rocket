#!/usr/bin/env node
// Writes the static `latest.json` the Tauri updater reads from
// https://github.com/xean-io/rocket/releases/latest/download/latest.json.
//
//   node scripts/updater-manifest.mjs --version v0.2.0 \
//     --url https://.../Rocket.app.tar.gz --sig-file Rocket.app.tar.gz.sig \
//     [--notes-file notes.md] [--pub-date 2026-10-06T12:00:00Z] [--out latest.json]
//
// The universal app runs natively on both architectures, so one archive and
// signature serve `darwin-aarch64` and `darwin-x86_64`. The signature file is
// the one `tauri build` writes next to the archive when `createUpdaterArtifacts`
// is on; it is public data (the private key never reaches this script).
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const DEFAULT_NOTES = "See the release notes on GitHub for what changed.";
const MAX_NOTES = 2000;
const TARGETS = ["darwin-aarch64", "darwin-x86_64"];

/** `v0.2.0` -> `0.2.0`; anything that is not a semantic version is an error. */
export function normalizeVersion(raw) {
  const v = String(raw ?? "").trim().replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(v)) {
    throw new Error(`invalid version ${JSON.stringify(raw)}: expected e.g. v0.2.0`);
  }
  return v;
}

function cleanNotes(notes) {
  const text = String(notes ?? "").trim();
  if (!text) return DEFAULT_NOTES;
  return text.length <= MAX_NOTES ? text : `${text.slice(0, MAX_NOTES - 1).trimEnd()}…`;
}

/** @returns the updater manifest object (see the Tauri updater "static JSON" format). */
export function buildManifest({ version, notes, pubDate = new Date(), url, signature }) {
  const sig = String(signature ?? "").trim();
  if (!sig) throw new Error("empty signature: was the app built with createUpdaterArtifacts?");
  if (!String(url ?? "").trim()) throw new Error("missing url of the updater archive");
  const platform = { url: url.trim(), signature: sig };
  return {
    version: normalizeVersion(version),
    notes: cleanNotes(notes),
    pub_date: new Date(pubDate).toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: Object.fromEntries(TARGETS.map((t) => [t, { ...platform }])),
  };
}

function parseFlags(argv) {
  const flags = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key?.startsWith("--") || argv[i + 1] === undefined) throw new Error(`bad argument ${key ?? ""}`);
    flags[key.slice(2)] = argv[i + 1];
  }
  return flags;
}

function readOptional(path) {
  if (!path) return undefined;
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

/** CLI entry: writes the manifest and returns it. Throws on bad input. */
export function main(argv) {
  const f = parseFlags(argv);
  for (const required of ["version", "url", "sig-file"]) {
    if (!f[required]) throw new Error(`missing --${required}`);
  }
  const manifest = buildManifest({
    version: f.version,
    url: f.url,
    signature: readFileSync(f["sig-file"], "utf8"),
    notes: readOptional(f["notes-file"]),
    pubDate: f["pub-date"] ? new Date(f["pub-date"]) : new Date(),
  });
  writeFileSync(f.out ?? "latest.json", `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`updater-manifest: ${e.message}`);
    process.exit(1);
  }
}
