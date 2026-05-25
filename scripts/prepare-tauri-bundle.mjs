import { copyFile, mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const root = process.cwd();
const tauriDir = join(root, "src-tauri");
const exeName = process.platform === "win32" ? "localmod-server.exe" : "localmod-server";
const serverOut = join(tauriDir, "target", "release", exeName);
const bundledServer = join(tauriDir, "binaries", exeName);
const runtimeDir = join(tauriDir, "binaries", "llama-runtime");

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

console.log("[bundle] Building headless API server...");
if (!existsSync(bundledServer)) {
  await mkdir(join(tauriDir, "binaries"), { recursive: true });
  await writeFile(
    bundledServer,
    "LocalMOD build placeholder. This file is replaced by scripts/prepare-tauri-bundle.mjs.\n",
  );
}
run("cargo", ["build", "--release", "--bin", "localmod-server"], tauriDir);

await mkdir(join(tauriDir, "binaries"), { recursive: true });
await copyFile(serverOut, bundledServer);
console.log(`[bundle] Copied ${exeName} into src-tauri/binaries/`);

if (!existsSync(runtimeDir)) {
  await mkdir(runtimeDir, { recursive: true });
}

const runtimeFiles = existsSync(runtimeDir)
  ? await import("node:fs/promises").then(({ readdir }) => readdir(runtimeDir))
  : [];
const hasRuntimeExe = runtimeFiles.some((name) =>
  process.platform === "win32"
    ? name.toLowerCase() === "llama-server.exe" || /^llama-server-.*\.exe$/i.test(name)
    : name === "llama-server" || name.startsWith("llama-server-"),
);

if (!hasRuntimeExe) {
  console.warn(
    "[bundle] WARNING: src-tauri/binaries/llama-runtime does not contain a real llama-server binary yet.",
  );
  console.warn(
    "[bundle] Add llama-server.exe and required DLLs there before shipping the installer.",
  );
}
