import type {
  ChecksumState,
  DownloadProgress,
  Drive,
  FlashEvent,
  FlashJob,
  ImageInfo,
} from "../types";
import { mockBackend } from "./mock";
import { tauriBackend } from "./tauri";

export type Unsubscribe = () => void;

/** Everything the UI needs from the outside world. */
export interface Backend {
  listDrives(showAll: boolean): Promise<Drive[]>;
  pickImage(): Promise<string | null>;
  pickDirectory(): Promise<string | null>;
  inspectImage(path: string): Promise<ImageInfo>;
  /** Hashes the image, reporting progress, and resolves with the final checksum state. */
  checksum(
    path: string,
    expected: string | null,
    onProgress: (s: ChecksumState) => void,
  ): Promise<ChecksumState>;
  download(
    url: string,
    dir: string | null,
    onProgress: (p: DownloadProgress) => void,
  ): Promise<string>;
  cancelDownload(): Promise<void>;
  flash(job: FlashJob, onEvent: (e: FlashEvent) => void): Promise<void>;
  cancelFlash(): Promise<void>;
  eject(driveId: string): Promise<void>;
  onFileDrop(cb: (path: string) => void): Unsubscribe;
}

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const backend: Backend = inTauri ? tauriBackend : mockBackend;
export const isMock = !inTauri;
