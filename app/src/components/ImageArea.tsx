import { useState, type DragEvent } from "react";
import { formatBytes, formatBytesOf, formatSpeed } from "../format";
import type { ImageSlot } from "../state";
import type { ChecksumState, FlashMode, ImageInfo } from "../types";
import { Alert, Check, Cross, Disc, Download, Windows } from "./icons";

interface Props {
  image: ImageSlot;
  checksum: ChecksumState;
  checksumOverride: boolean;
  mode: FlashMode | null;
  onBrowse: () => void;
  onPasteLink: () => void;
  onReplace: () => void;
  onCancelDownload: () => void;
  onPasteChecksum: () => void;
  onOverrideChecksum: () => void;
  onChangeMode: () => void;
}

export function ImageArea(p: Props) {
  switch (p.image.kind) {
    case "none":
    case "error":
      return (
        <DropZone
          error={p.image.kind === "error" ? p.image.message : null}
          onBrowse={p.onBrowse}
          onPasteLink={p.onPasteLink}
        />
      );
    case "loading":
      return (
        <div className="image-card">
          <div className="image-icon">
            <Disc />
          </div>
          <div className="image-body">
            <div className="image-name">{p.image.name}</div>
            <div className="image-meta">Reading image…</div>
          </div>
        </div>
      );
    case "downloading": {
      const prog = p.image.progress;
      const name = p.image.url.split("/").pop() || p.image.url;
      const pct = prog?.total ? (prog.bytes / prog.total) * 100 : null;
      return (
        <div className="image-card">
          <div className="image-icon">
            <Download />
          </div>
          <div className="image-body">
            <div className="image-name">{name}</div>
            <div className="image-meta">
              {prog
                ? `${prog.total ? formatBytesOf(prog.bytes, prog.total) : formatBytes(prog.bytes)} · ${formatSpeed(prog.speed)}`
                : "Connecting…"}
            </div>
            <div className="mini-bar">
              <div
                className={pct == null ? "mini-bar-fill indeterminate" : "mini-bar-fill"}
                style={pct == null ? undefined : { width: `${pct}%` }}
              />
            </div>
          </div>
          <button className="btn-quiet" onClick={p.onCancelDownload}>
            Cancel
          </button>
        </div>
      );
    }
    case "ready":
      return <ReadyCard {...p} info={p.image.info} />;
  }
}

function DropZone({
  error,
  onBrowse,
  onPasteLink,
}: {
  error: string | null;
  onBrowse: () => void;
  onPasteLink: () => void;
}) {
  const [over, setOver] = useState(false);
  // Real drops are delivered by Tauri's file-drop event; this only drives the hover look.
  const dragProps = {
    onDragEnter: (e: DragEvent) => {
      e.preventDefault();
      setOver(true);
    },
    onDragOver: (e: DragEvent) => e.preventDefault(),
    onDragLeave: () => setOver(false),
    onDrop: (e: DragEvent) => {
      e.preventDefault();
      setOver(false);
    },
  };
  return (
    <div className={over ? "drop-zone over" : "drop-zone"} {...dragProps}>
      <Disc size={34} className="accent" />
      <div className="drop-title">Drop an .iso or .img here</div>
      <div className="drop-actions">
        <button className="btn-dark" onClick={onBrowse}>
          Browse files
        </button>
        <span>
          or{" "}
          <button className="link" onClick={onPasteLink}>
            paste a download link
          </button>
        </span>
      </div>
      {error && (
        <div className="drop-error">
          <Alert size={14} /> {error}
        </div>
      )}
    </div>
  );
}

function describe(info: ImageInfo, mode: FlashMode | null): string {
  const parts: string[] = [];
  if (info.compression !== "none") {
    parts.push(
      info.writeSize
        ? `${formatBytes(info.fileSize)} ${info.compression} → ${formatBytes(info.writeSize)}`
        : `${formatBytes(info.fileSize)} ${info.compression}`,
    );
  } else {
    parts.push(formatBytes(info.fileSize));
  }
  parts.push(info.format);
  if (info.boot) parts.push(info.boot);
  if (mode === "windows") parts.push("Windows installer");
  return parts.join(" · ");
}

function ReadyCard(p: Props & { info: ImageInfo }) {
  const { info, checksum, mode } = p;
  return (
    <div className="image-card">
      <div className="image-icon">{mode === "windows" ? <Windows /> : <Disc />}</div>
      <div className="image-body">
        <div className="image-name" title={info.path}>
          {info.name}
        </div>
        <div className="image-meta">
          {describe(info, mode)}
          {info.kind === "unknown" && mode && (
            <>
              {" · "}
              <button className="link" onClick={p.onChangeMode}>
                {mode === "raw" ? "written raw" : "change"}
              </button>
            </>
          )}
        </div>
        {info.compression !== "none" && !info.writeSize && (
          <div className="image-note">
            Uncompressed size unknown. If it's bigger than the drive, the write fails partway.
          </div>
        )}
        <ChecksumLine
          state={checksum}
          override={p.checksumOverride}
          onPaste={p.onPasteChecksum}
          onOverride={p.onOverrideChecksum}
        />
      </div>
      <button className="btn-outline" onClick={p.onReplace}>
        Replace
      </button>
    </div>
  );
}

function ChecksumLine({
  state,
  override,
  onPaste,
  onOverride,
}: {
  state: ChecksumState;
  override: boolean;
  onPaste: () => void;
  onOverride: () => void;
}) {
  switch (state.status) {
    case "idle":
      return null;
    case "hashing":
      return (
        <div className="checksum muted">
          Checking SHA-256… {state.total ? Math.floor((state.bytes / state.total) * 100) : 0}%
        </div>
      );
    case "match":
      return (
        <div className="checksum ok">
          <Check />
          {state.source === "pasted"
            ? "SHA-256 matches the checksum you pasted"
            : "SHA-256 matches the published checksum"}
        </div>
      );
    case "mismatch":
      return (
        <div className="checksum bad">
          <Cross />
          {override ? "Checksum mismatch overridden" : `SHA-256 doesn't match ${state.source === "pasted" ? "the checksum you pasted" : state.source}`}
          {!override && (
            <>
              <button className="link" onClick={onPaste}>
                Paste another
              </button>
              <button className="link danger" onClick={onOverride}>
                Flash anyway
              </button>
            </>
          )}
        </div>
      );
    case "unverified":
      return (
        <div className="checksum muted">
          No published checksum found ·{" "}
          <button className="link" onClick={onPaste}>
            Paste one
          </button>
        </div>
      );
  }
}
