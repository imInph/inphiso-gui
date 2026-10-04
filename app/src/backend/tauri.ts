import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type { Backend } from ".";
import type { ChecksumState, DownloadProgress, FlashEvent } from "../types";

const IMAGE_EXTENSIONS = ["iso", "img", "raw", "dd", "usb", "xz", "gz", "bz2", "zst"];

export const tauriBackend: Backend = {
  listDrives: (showAll) => invoke("list_drives", { showAll }),

  async pickImage() {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Disk images", extensions: IMAGE_EXTENSIONS }],
    });
    return typeof picked === "string" ? picked : null;
  },

  async pickDirectory() {
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  },

  inspectImage: (path) => invoke("inspect_image", { path }),

  checksum(path, expected, onProgress) {
    const channel = new Channel<ChecksumState>();
    channel.onmessage = onProgress;
    return invoke("checksum", { path, expected, onProgress: channel });
  },

  download(url, dir, onProgress) {
    const channel = new Channel<DownloadProgress>();
    channel.onmessage = onProgress;
    return invoke("download", { url, dir, onProgress: channel });
  },

  cancelDownload: () => invoke("cancel_download"),

  flash(job, onEvent) {
    const channel = new Channel<FlashEvent>();
    channel.onmessage = onEvent;
    return invoke("flash", { job, onEvent: channel });
  },

  cancelFlash: () => invoke("cancel_flash"),

  eject: (driveId) => invoke("eject", { driveId }),

  onFileDrop(cb) {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop" && event.payload.paths.length > 0) {
          cb(event.payload.paths[0]);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  },
};
