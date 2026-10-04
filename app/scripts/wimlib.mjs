// Fetches wimlib-imagex for a release build and puts it where the
// `tauri.release.<os>.json` overlays expect it:
//   src-tauri/binaries/wimlib-imagex-<triple>[.exe]
//   src-tauri/binaries/libwim-15.dll          (Windows only)
//   src-tauri/binaries/wimlib-licenses/        (license texts shipped with the app)
//
// Windows uses wimlib's official builds; macOS builds it from source, statically.
// Linux packages depend on the distro's wimtools instead, so this does nothing there.
//
// Usage: node scripts/wimlib.mjs <target-triple>
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "1.14.5";
const BASE = "https://wimlib.net/downloads";
const SHA256 = {
  [`wimlib-${VERSION}.tar.gz`]: "84221a3abd5b91228f15f8e6065c335a336237b5738197b75bf419eea561a194",
  [`wimlib-${VERSION}-windows-x86_64-bin.zip`]: "2f446d6fa3866582175f1a22a7be198eeee0aec7aba5b4e04ad25c99eae2d265",
  [`wimlib-${VERSION}-windows-aarch64-bin.zip`]: "2c2e3e50cf21bfdd4ffd693b5052b231aa04a10c85f218ae61eb5a031dfcdd8f",
};

const here = dirname(fileURLToPath(import.meta.url));
const binaries = join(here, "..", "src-tauri", "binaries");
const licenses = join(binaries, "wimlib-licenses");
const triple = process.argv[2];
if (!triple) {
  console.error("usage: node scripts/wimlib.mjs <target-triple>");
  process.exit(2);
}

const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", ...opts });

async function download(name) {
  const dest = join(tmpdir(), name);
  if (!existsSync(dest)) {
    console.log(`wimlib: downloading ${name}`);
    const res = await fetch(`${BASE}/${name}`);
    if (!res.ok) throw new Error(`${name}: HTTP ${res.status}`);
    writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
  }
  const actual = createHash("sha256").update(readFileSync(dest)).digest("hex");
  if (actual !== SHA256[name]) {
    rmSync(dest);
    throw new Error(`${name}: checksum mismatch (got ${actual})`);
  }
  return dest;
}

function copyLicenses(fromDir) {
  mkdirSync(licenses, { recursive: true });
  for (const f of readdirSync(fromDir).filter((f) => f.startsWith("COPYING"))) {
    copyFileSync(join(fromDir, f), join(licenses, f.endsWith(".txt") ? f : `${f}.txt`));
  }
  writeFileSync(
    join(licenses, "SOURCE.txt"),
    `wimlib-imagex ${VERSION} (GPLv3+, library LGPLv3+) is shipped unmodified.\n` +
      `Source code: ${BASE}/wimlib-${VERSION}.tar.gz\nProject: https://wimlib.net\n`,
  );
}

async function windows(arch) {
  const zip = await download(`wimlib-${VERSION}-windows-${arch}-bin.zip`);
  const out = join(tmpdir(), `wimlib-${VERSION}-${arch}`);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  // bsdtar (tar.exe on Windows 10+) reads zip files.
  run("tar", ["-xf", zip, "-C", out]);
  copyFileSync(join(out, "wimlib-imagex.exe"), join(binaries, `wimlib-imagex-${triple}.exe`));
  copyFileSync(join(out, "libwim-15.dll"), join(binaries, "libwim-15.dll"));
  copyLicenses(out);
}

async function macos(arch) {
  const tarball = await download(`wimlib-${VERSION}.tar.gz`);
  const work = join(tmpdir(), `wimlib-build-${arch}`);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  run("tar", ["-xzf", tarball, "-C", work]);
  const src = join(work, `wimlib-${VERSION}`);
  const host = arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  const env = {
    ...process.env,
    CFLAGS: `-arch ${arch} -O2 -mmacosx-version-min=11.0`,
    LDFLAGS: `-arch ${arch} -mmacosx-version-min=11.0`,
    // No optional dependencies are enabled, so pkg-config isn't needed.
    PKG_CONFIG: process.env.PKG_CONFIG || "true",
  };
  run(
    "./configure",
    ["--host", host, "--disable-shared", "--enable-static", "--without-fuse", "--without-ntfs-3g", "--quiet"],
    { cwd: src, env },
  );
  run("make", ["-j4", "wimlib-imagex"], { cwd: src, env });
  copyFileSync(join(src, "wimlib-imagex"), join(binaries, `wimlib-imagex-${triple}`));
  copyLicenses(src);
}

mkdirSync(binaries, { recursive: true });
if (triple === "x86_64-pc-windows-msvc") await windows("x86_64");
else if (triple === "aarch64-pc-windows-msvc") await windows("aarch64");
else if (triple === "aarch64-apple-darwin") await macos("arm64");
else if (triple === "x86_64-apple-darwin") await macos("x86_64");
else console.log(`wimlib: nothing to do for ${triple} (Linux uses the distro's wimtools)`);
