// Build the Agent and CLI and place them where Tauri's `externalBin` expects
// them: src-tauri/binaries/<name>-<target-triple>[.exe].
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", "..");
const profile = process.argv.includes("--debug") ? "debug" : "release";
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1].trim();
const ext = triple.includes("windows") ? ".exe" : "";

const args = ["build", "-p", "continuity-agent", "-p", "continuity-cli", "--target", triple];
if (profile === "release") args.push("--release");
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const outDir = join(here, "..", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
for (const name of ["continuity-agent", "continuity"]) {
  const src = join(root, "target", triple, profile, name + ext);
  const dst = join(outDir, `${name}-${triple}${ext}`);
  copyFileSync(src, dst);
  console.log(`sidecar: ${dst}`);
}
