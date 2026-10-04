// Builds inphiso-helper for the target Tauri is building, and puts it where
// `bundle.externalBin` expects it: src-tauri/binaries/inphiso-helper-<triple>.
// Runs before `tauri dev` / `tauri build`; also run it before `cargo clippy`.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)/m)[1];
const positional = process.argv.slice(2).find((a) => !a.startsWith("--"));
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || positional || host;
const debug = process.env.TAURI_ENV_DEBUG === "true" || process.argv.includes("--debug");
const ext = triple.includes("windows") ? ".exe" : "";

const args = ["build", "-p", "inphiso-helper", "--target", triple];
if (!debug) args.push("--release");
console.log(`sidecar: cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const built = join(root, "target", triple, debug ? "debug" : "release", `inphiso-helper${ext}`);
const dest = join(root, "app", "src-tauri", "binaries", `inphiso-helper-${triple}${ext}`);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(built, dest);
console.log(`sidecar: ${dest}`);
