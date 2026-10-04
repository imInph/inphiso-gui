// A fake backend so the UI can be developed and previewed in a plain browser.
import type { Backend } from ".";
import type { ChecksumState, Drive, FlashEvent, ImageInfo } from "../types";

const GB = 1e9;

const drives: Drive[] = [
  { id: "disk4", name: "SanDisk Ultra", size: 32 * GB, bus: "USB 3.0", removable: true, volumes: ["UNTITLED"] },
  { id: "disk5", name: "Samsung T7", size: 1000 * GB, bus: "USB-C", removable: true, volumes: ["T7", "Backups"] },
  { id: "disk6", name: "Kingston DataTraveler", size: 4 * GB, bus: "USB 2.0", removable: true, volumes: [] },
];

const internal: Drive = {
  id: "disk2",
  name: "WD Black SN770",
  size: 2000 * GB,
  bus: "Thunderbolt",
  removable: false,
  volumes: ["Media"],
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

let cancelled = false;

export const mockBackend: Backend = {
  async listDrives(showAll) {
    await sleep(80);
    return showAll ? [...drives, internal] : drives;
  },

  async pickImage() {
    return "/Users/you/Downloads/ubuntu-24.04.1-desktop-amd64.iso";
  },

  async pickDirectory() {
    return "/Users/you/Images";
  },

  async inspectImage(path): Promise<ImageInfo> {
    await sleep(150);
    const name = path.split(/[\\/]/).pop() ?? path;
    const windows = /win/i.test(name);
    return {
      path,
      name,
      fileSize: windows ? 5.8 * GB : 6.1 * GB,
      writeSize: windows ? 5.8 * GB : 6.1 * GB,
      compression: name.endsWith(".xz") ? "xz" : "none",
      kind: windows ? "windows" : /\.img/.test(name) ? "unknown" : "raw",
      format: windows ? "UDF" : "ISO 9660",
      boot: windows ? "bootable (UEFI)" : "bootable (UEFI + BIOS)",
    };
  },

  async checksum(_path, expected, onProgress): Promise<ChecksumState> {
    const total = 6.1 * GB;
    for (let i = 1; i <= 10; i++) {
      await sleep(90);
      onProgress({ status: "hashing", bytes: (total * i) / 10, total });
    }
    const actual = "c2e6f4dc9b0e2f7a51d3c8e4b6a90f12d7e5c3b1a8f6e4d2c0b9a7f5e3d1a1f3";
    if (expected && expected.toLowerCase() !== actual) {
      return { status: "mismatch", actual, expected, source: "pasted" };
    }
    if (expected) return { status: "match", actual, source: "pasted" };
    return { status: "match", actual, source: "SHA256SUMS" };
  },

  async download(url, _dir, onProgress) {
    cancelled = false;
    const total = 2.4 * GB;
    for (let i = 1; i <= 20; i++) {
      if (cancelled) throw new Error("Download cancelled");
      await sleep(100);
      onProgress({ bytes: (total * i) / 20, total, speed: 48e6 });
    }
    return "/Users/you/Downloads/" + (url.split("/").pop() || "image.iso");
  },

  async cancelDownload() {
    cancelled = true;
  },

  async flash(job, onEvent: (e: FlashEvent) => void) {
    cancelled = false;
    const total = 6.1 * GB;
    const start = Date.now();
    let peak = 0;
    const phases = job.verify ? (["write", "verify"] as const) : (["write"] as const);
    for (const phase of phases) {
      const steps = phase === "write" ? 60 : 25;
      for (let i = 1; i <= steps; i++) {
        if (cancelled) {
          onEvent({ type: "cancelled" });
          return;
        }
        await sleep(70);
        const speed = (phase === "write" ? 34e6 : 90e6) + Math.sin(i / 2) * 5e6;
        peak = Math.max(peak, speed);
        const bytes = (total * i) / steps;
        onEvent({
          type: "progress",
          progress: { phase, bytes, total, speed, peak, etaSecs: Math.ceil((total - bytes) / speed) },
        });
      }
    }
    onEvent({ type: "done", elapsedMs: Date.now() - start, verified: job.verify });
  },

  async cancelFlash() {
    cancelled = true;
  },

  async eject() {
    await sleep(200);
  },

  onFileDrop() {
    return () => {};
  },
};
