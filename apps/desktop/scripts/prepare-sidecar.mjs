#!/usr/bin/env node
// Builds the `rocket` CLI and places it where Tauri's `bundle.externalBin`
// expects it: src-tauri/binaries/rocket-<target-triple>[.exe].
//
// Target triple, first match wins: `--target <triple>`, $TAURI_ENV_TARGET_TRIPLE
// (set by `tauri build/dev` for the before*Command hooks), the rustc host.
// `universal-apple-darwin` builds both macOS architectures and joins them
// with lipo, which is the file name Tauri looks for in universal bundles.
//
// Flags: `--debug` builds the debug profile (used by `tauri dev`).
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const outDir = resolve(here, "../src-tauri/binaries");

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

const run = (cmd, cmdArgs, env = {}) =>
  execFileSync(cmd, cmdArgs, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, ...env },
  });

const hostTriple = () => {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = /^host: (.+)$/m.exec(out);
  if (!m) throw new Error("cannot read the host triple from `rustc -vV`");
  return m[1];
};

const triple =
  option("--target") || process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const profile = flag("--debug") ? "debug" : "release";
const exe = triple.includes("windows") ? ".exe" : "";

// `rocket version` reports the release version when ROCKET_VERSION is set.
const env = {};
if (process.env.ROCKET_VERSION) env.ROCKET_VERSION = process.env.ROCKET_VERSION;

const build = (target) => {
  const cargoArgs = ["build", "-p", "rocket-cli", "--locked", "--target", target];
  if (profile === "release") cargoArgs.push("--release");
  run("cargo", cargoArgs, env);
  return join(root, "target", target, profile, `rocket${exe}`);
};

mkdirSync(outDir, { recursive: true });
const place = (target, source) => {
  const dest = join(outDir, `rocket-${target}${exe}`);
  copyFileSync(source, dest);
  chmodSync(dest, 0o755);
  console.log(`sidecar ready: ${dest}`);
  return dest;
};

if (triple === "universal-apple-darwin") {
  // `tauri build --target universal-apple-darwin` compiles each architecture
  // separately (each needs its own `rocket-<triple>` file, since tauri-build
  // checks it) and then bundles the universal one.
  const parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"].map((t) =>
    place(t, build(t)),
  );
  const joined = join(outDir, "rocket-universal-apple-darwin");
  run("lipo", ["-create", "-output", joined, ...parts]);
  chmodSync(joined, 0o755);
  console.log(`sidecar ready: ${joined}`);
} else {
  place(triple, build(triple));
}
