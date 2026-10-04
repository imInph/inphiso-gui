// Shapes shared with the Rust side. Field names match the serde output.

export interface Drive {
  id: string;
  name: string;
  size: number;
  bus: string | null;
  removable: boolean;
  volumes: string[];
}

export type Compression = "none" | "xz" | "gzip" | "bzip2" | "zstd";

/** How the image will be written. `unknown` means we have to ask the user. */
export type ImageKind = "raw" | "windows" | "unknown";

export interface ImageInfo {
  path: string;
  name: string;
  /** Size of the file on disk. */
  fileSize: number;
  /** Bytes that end up on the drive, if known (null for gzip/bzip2/zstd). */
  writeSize: number | null;
  compression: Compression;
  kind: ImageKind;
  /** Short format description: "ISO 9660", "UDF", "raw disk image". */
  format: string;
  /** e.g. "bootable (UEFI + BIOS)", or null if we can't tell. */
  boot: string | null;
}

export type ChecksumState =
  | { status: "idle" }
  | { status: "hashing"; bytes: number; total: number }
  | { status: "match"; actual: string; source: string }
  | { status: "mismatch"; actual: string; expected: string; source: string }
  | { status: "unverified"; actual: string };

export type FlashMode = "raw" | "windows";

export interface FlashJob {
  imagePath: string;
  driveId: string;
  driveSize: number;
  mode: FlashMode;
  verify: boolean;
  partitionScheme: "mbr" | "gpt";
}

export type Phase = "write" | "verify";

export interface Progress {
  phase: Phase;
  bytes: number;
  total: number | null;
  speed: number;
  peak: number;
  etaSecs: number | null;
}

export type FlashEvent =
  | { type: "progress"; progress: Progress }
  | { type: "done"; elapsedMs: number; verified: boolean }
  | { type: "error"; message: string }
  | { type: "cancelled" };

export interface DownloadProgress {
  bytes: number;
  total: number | null;
  speed: number;
}

export interface Settings {
  showAllDrives: boolean;
  verify: boolean;
  partitionScheme: "mbr" | "gpt";
  ejectWhenDone: boolean;
  downloadDir: string | null;
}

export const defaultSettings: Settings = {
  showAllDrives: false,
  verify: true,
  partitionScheme: "mbr",
  ejectWhenDone: false,
  downloadDir: null,
};
