import type {
  ChecksumState,
  DownloadProgress,
  Drive,
  FlashEvent,
  FlashMode,
  ImageInfo,
  Progress,
} from "./types";

export type ImageSlot =
  | { kind: "none" }
  | { kind: "loading"; name: string }
  | { kind: "downloading"; url: string; progress: DownloadProgress | null }
  | { kind: "ready"; info: ImageInfo }
  | { kind: "error"; message: string };

export type Screen = "select" | "writing" | "done" | "failed" | "cancelled";

export type Modal = null | "confirm" | "imageType" | "settings" | "link" | "checksum";

export interface AppState {
  screen: Screen;
  image: ImageSlot;
  checksum: ChecksumState;
  /** Hash the user pasted, if any. */
  expected: string | null;
  /** User chose to flash despite a checksum mismatch. */
  checksumOverride: boolean;
  drives: Drive[];
  /** False until the first drive list arrives, so we don't flash "no drives". */
  drivesLoaded: boolean;
  pickedDriveId: string | null;
  /** The drive being (or last) flashed, as it was when the flash started. Drives
   *  vanish from the list when ejected, so later screens must not look it up there. */
  flashedDrive: Drive | null;
  /** Write mode, resolved from the image or asked of the user. */
  mode: FlashMode | null;
  progress: Progress | null;
  /** Throughput samples for the chart: [fraction of phase done, bytes/s]. Reset per phase. */
  speeds: Array<[number, number]>;
  result: { elapsedMs: number; verified: boolean } | null;
  error: string | null;
  modal: Modal;
}

export const initialState: AppState = {
  screen: "select",
  image: { kind: "none" },
  checksum: { status: "idle" },
  expected: null,
  checksumOverride: false,
  drives: [],
  drivesLoaded: false,
  pickedDriveId: null,
  flashedDrive: null,
  mode: null,
  progress: null,
  speeds: [],
  result: null,
  error: null,
  modal: null,
};

export type Action =
  | { type: "imageLoading"; name: string }
  | { type: "imageReady"; info: ImageInfo }
  | { type: "imageError"; message: string }
  | { type: "clearImage" }
  | { type: "downloadStart"; url: string }
  | { type: "downloadProgress"; progress: DownloadProgress }
  | { type: "checksum"; state: ChecksumState }
  | { type: "setExpected"; hash: string | null }
  | { type: "overrideChecksum" }
  | { type: "drives"; drives: Drive[] }
  | { type: "pickDrive"; id: string }
  | { type: "setMode"; mode: FlashMode }
  | { type: "modal"; modal: Modal }
  | { type: "flashStart"; drive: Drive }
  | { type: "flashEvent"; event: FlashEvent }
  | { type: "flashFailed"; message: string }
  | { type: "backToSelect" }
  | { type: "flashAnother" };

const MAX_SPEED_SAMPLES = 120;

/** Bytes that will land on the drive; falls back to the file size when unknown. */
export function requiredSize(info: ImageInfo): number {
  return info.writeSize ?? info.fileSize;
}

export function driveFits(drive: Drive, image: ImageSlot): boolean {
  return image.kind !== "ready" || drive.size >= requiredSize(image.info);
}

export function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case "imageLoading":
      return {
        ...state,
        image: { kind: "loading", name: action.name },
        checksum: { status: "idle" },
        checksumOverride: false,
        mode: null,
      };

    case "imageReady": {
      const info = action.info;
      const mode = info.kind === "unknown" ? null : info.kind;
      const picked = state.drives.find((d) => d.id === state.pickedDriveId);
      const keepPick = picked && picked.size >= requiredSize(info);
      return {
        ...state,
        image: { kind: "ready", info },
        mode,
        pickedDriveId: keepPick ? state.pickedDriveId : autoPick(state.drives, info),
        modal: info.kind === "unknown" ? "imageType" : state.modal,
      };
    }

    case "imageError":
      return { ...state, image: { kind: "error", message: action.message } };

    case "clearImage":
      return {
        ...state,
        image: { kind: "none" },
        checksum: { status: "idle" },
        expected: null,
        checksumOverride: false,
        mode: null,
      };

    case "downloadStart":
      return {
        ...state,
        image: { kind: "downloading", url: action.url, progress: null },
        checksum: { status: "idle" },
        modal: null,
      };

    case "downloadProgress":
      if (state.image.kind !== "downloading") return state;
      return { ...state, image: { ...state.image, progress: action.progress } };

    case "checksum":
      return { ...state, checksum: action.state };

    case "setExpected":
      return { ...state, expected: action.hash, checksumOverride: false, modal: null };

    case "overrideChecksum":
      return { ...state, checksumOverride: true };

    case "drives": {
      // Only the picker screen follows hotplug. During and after a flash the
      // selection stays put, so an ejected stick never gets swapped for another drive.
      if (state.screen !== "select") {
        return { ...state, drives: action.drives, drivesLoaded: true };
      }
      const stillThere = action.drives.some((d) => d.id === state.pickedDriveId);
      let pickedDriveId = stillThere ? state.pickedDriveId : null;
      if (!pickedDriveId && state.image.kind === "ready") {
        pickedDriveId = autoPick(action.drives, state.image.info);
      }
      return { ...state, drives: action.drives, drivesLoaded: true, pickedDriveId };
    }

    case "pickDrive":
      return { ...state, pickedDriveId: action.id };

    case "setMode":
      return { ...state, mode: action.mode, modal: null };

    case "modal":
      return { ...state, modal: action.modal };

    case "flashStart":
      return {
        ...state,
        flashedDrive: action.drive,
        screen: "writing",
        modal: null,
        progress: null,
        speeds: [],
        result: null,
        error: null,
      };

    case "flashEvent": {
      const e = action.event;
      switch (e.type) {
        case "progress": {
          const phaseChanged = state.progress?.phase !== e.progress.phase;
          const speeds = phaseChanged ? [] : state.speeds;
          const { bytes, total, speed } = e.progress;
          if (!total) {
            // gzip/bzip2/zstd: no total, so show a rolling window of recent samples.
            const recent = [...speeds.map(([, s]) => s), speed].slice(-MAX_SPEED_SAMPLES);
            const pts = recent.map((s, i): [number, number] => [i / (MAX_SPEED_SAMPLES - 1), s]);
            return { ...state, progress: e.progress, speeds: pts };
          }
          const x = Math.min(bytes / total, 1);
          const lastX = speeds.length ? speeds[speeds.length - 1][0] : -1;
          // One point per 0.5% keeps the chart to ~200 points however chatty the helper is.
          const next = x - lastX >= 0.005 ? [...speeds, [x, speed] as [number, number]] : speeds;
          return { ...state, progress: e.progress, speeds: next };
        }
        case "done":
          return { ...state, screen: "done", result: { elapsedMs: e.elapsedMs, verified: e.verified } };
        case "error":
          return { ...state, screen: "failed", error: e.message };
        case "cancelled":
          return { ...state, screen: "cancelled" };
      }
      return state;
    }

    case "flashFailed":
      return { ...state, screen: "failed", error: action.message };

    case "backToSelect": {
      // The drive may have been unplugged meanwhile; then the user picks again.
      const stillThere = state.drives.some((d) => d.id === state.pickedDriveId);
      return {
        ...state,
        screen: "select",
        progress: null,
        speeds: [],
        error: null,
        pickedDriveId: stillThere ? state.pickedDriveId : null,
      };
    }

    case "flashAnother":
      return { ...initialState, drives: state.drives, drivesLoaded: state.drivesLoaded };
  }
}

/** Pre-select the drive when exactly one removable drive fits. */
function autoPick(drives: Drive[], info: ImageInfo): string | null {
  const fits = drives.filter((d) => d.removable && d.size >= requiredSize(info));
  return fits.length === 1 ? fits[0].id : null;
}

/** Whether the Flash button can be pressed, and if not, why. */
export function flashBlocker(state: AppState): string | null {
  if (state.image.kind !== "ready") return "Pick an image to see which drives fit.";
  if (state.checksum.status === "hashing") return "Checking the image…";
  if (state.checksum.status === "mismatch" && !state.checksumOverride)
    return "The checksum doesn't match. Re-download the image or override.";
  if (!state.mode) return "Choose how to write this image.";
  if (!state.pickedDriveId) return "Pick a drive.";
  return null;
}
