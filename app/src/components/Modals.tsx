import { useEffect, useRef, useState, type ReactNode } from "react";
import { formatBytes } from "../format";
import type { Drive, FlashMode, ImageInfo, Settings } from "../types";
import { Alert, Disc, Windows } from "./icons";

function Modal({
  title,
  onClose,
  children,
  wide,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={wide ? "dialog wide" : "dialog"} role="dialog" aria-modal="true" aria-label={title}>
        <div className="dialog-title">{title}</div>
        {children}
      </div>
    </div>
  );
}

const ARM_DELAY_MS = 2000;

export function ConfirmModal({
  drive,
  image,
  mode,
  scheme,
  onCancel,
  onConfirm,
}: {
  drive: Drive;
  image: ImageInfo;
  mode: FlashMode;
  scheme: "mbr" | "gpt";
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [armed, setArmed] = useState(false);
  const [typed, setTyped] = useState("");
  // Internal drives only appear with "Show all drives"; make those deliberate.
  const mustType = !drive.removable;
  useEffect(() => {
    const t = setTimeout(() => setArmed(true), ARM_DELAY_MS);
    return () => clearTimeout(t);
  }, []);
  const ready = armed && (!mustType || typed.trim() === drive.name);
  return (
    <Modal title={`Erase ${drive.name}?`} onClose={onCancel}>
      <div className="confirm-warning">
        <Alert size={16} />
        Everything on this drive will be erased. This can't be undone.
      </div>
      <div className="summary in-dialog">
        <div className="summary-row">
          <span className="muted">Drive</span>
          <span>
            {drive.name} · {formatBytes(drive.size)}
          </span>
        </div>
        <div className="summary-row">
          <span className="muted">Device</span>
          <span className="mono">{drive.id}</span>
        </div>
        <div className="summary-row">
          <span className="muted">Volumes</span>
          <span>{drive.volumes.length ? drive.volumes.join(", ") : "none"}</span>
        </div>
        <div className="summary-row">
          <span className="muted">Image</span>
          <span className="mono ellipsis">{image.name}</span>
        </div>
        <div className="summary-row">
          <span className="muted">Method</span>
          <span>
            {mode === "raw"
              ? "Byte-for-byte copy"
              : `Windows installer · FAT32 · ${scheme.toUpperCase()} · ${scheme === "mbr" ? "UEFI + BIOS" : "UEFI"}`}
          </span>
        </div>
      </div>
      {mustType && (
        <label className="type-confirm">
          <span>
            This is an internal drive. Type <b>{drive.name}</b> to confirm.
          </span>
          <input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} spellCheck={false} />
        </label>
      )}
      <div className="dialog-actions">
        <button className="btn-outline lg" onClick={onCancel}>
          Cancel
        </button>
        <button className="btn-danger lg" disabled={!ready} onClick={onConfirm}>
          Erase and flash
          {!armed && <span className="arming" />}
        </button>
      </div>
    </Modal>
  );
}

export function ImageTypeModal({
  current,
  onPick,
  onClose,
}: {
  current: FlashMode | null;
  onPick: (m: FlashMode) => void;
  onClose: () => void;
}) {
  return (
    <Modal title="How should this image be written?" onClose={onClose} wide>
      <p className="dialog-text">
        inphiso couldn't tell whether this is a Windows installer or a Linux / raw image.
      </p>
      <div className="choice-grid">
        <button className={current === "raw" ? "choice on" : "choice"} onClick={() => onPick("raw")}>
          <Disc size={26} />
          <div className="choice-title">Write as-is</div>
          <div className="choice-text">
            For Linux ISOs and disk images (.img). Copies the image to the drive byte for byte.
          </div>
        </button>
        <button className={current === "windows" ? "choice on" : "choice"} onClick={() => onPick("windows")}>
          <Windows size={26} />
          <div className="choice-title">Windows installer</div>
          <div className="choice-text">
            Formats the drive as FAT32 and copies the files, splitting install.wim if it's over 4 GB.
          </div>
        </button>
      </div>
    </Modal>
  );
}

export function LinkModal({ onSubmit, onClose }: { onSubmit: (url: string) => void; onClose: () => void }) {
  const [url, setUrl] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);
  const valid = /^https:\/\/[^\s/]+\.[^\s]+$/i.test(url.trim());
  return (
    <Modal title="Download an image" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) onSubmit(url.trim());
        }}
      >
        <input
          ref={inputRef}
          className="text-input mono"
          placeholder="https://releases.ubuntu.com/…/ubuntu-24.04.1-desktop-amd64.iso"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          spellCheck={false}
        />
        <p className="dialog-hint">
          HTTPS links only. The image is saved to your download folder, then checked against a published
          SHA-256 if one sits next to it.
        </p>
        <div className="dialog-actions">
          <button type="button" className="btn-outline lg" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn-primary lg" disabled={!valid}>
            Download
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function ChecksumModal({
  initial,
  onSubmit,
  onClose,
}: {
  initial: string | null;
  onSubmit: (hash: string) => void;
  onClose: () => void;
}) {
  const [hash, setHash] = useState(initial ?? "");
  // Accept a whole "hash  filename" line pasted from a SHA256SUMS file.
  const cleaned = hash.trim().split(/\s+/)[0]?.toLowerCase() ?? "";
  const valid = /^[0-9a-f]{64}$/.test(cleaned);
  return (
    <Modal title="Compare with a SHA-256" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) onSubmit(cleaned);
        }}
      >
        <input
          autoFocus
          className="text-input mono"
          placeholder="64 hex characters"
          value={hash}
          onChange={(e) => setHash(e.target.value)}
          spellCheck={false}
        />
        <p className="dialog-hint">Copy it from the download page of the image you're flashing.</p>
        <div className="dialog-actions">
          <button type="button" className="btn-outline lg" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn-primary lg" disabled={!valid}>
            Compare
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function SettingsModal({
  settings,
  version,
  onChange,
  onPickDownloadDir,
  onClose,
}: {
  settings: Settings;
  version: string;
  onChange: (s: Partial<Settings>) => void;
  onPickDownloadDir: () => void;
  onClose: () => void;
}) {
  return (
    <Modal title="Settings" onClose={onClose} wide>
      <div className="settings">
        <Toggle
          label="Show all drives"
          hint="Lists internal and non-USB drives too. Your system disk is never shown."
          checked={settings.showAllDrives}
          onChange={(v) => onChange({ showAllDrives: v })}
        />
        <Toggle
          label="Verify after writing"
          hint="Reads the drive back and compares it with the image. Takes a bit longer."
          checked={settings.verify}
          onChange={(v) => onChange({ verify: v })}
        />
        <Toggle
          label="Eject when done"
          checked={settings.ejectWhenDone}
          onChange={(v) => onChange({ ejectWhenDone: v })}
        />
        <div className="setting">
          <div>
            <div className="setting-label">Windows partition scheme</div>
            <div className="setting-hint">MBR boots on both UEFI and legacy BIOS PCs. GPT is UEFI only.</div>
          </div>
          <div className="segmented">
            {(["mbr", "gpt"] as const).map((s) => (
              <button
                key={s}
                className={settings.partitionScheme === s ? "on" : undefined}
                onClick={() => onChange({ partitionScheme: s })}
              >
                {s.toUpperCase()}
              </button>
            ))}
          </div>
        </div>
        <div className="setting">
          <div className="setting-grow">
            <div className="setting-label">Download folder</div>
            <div className="setting-hint mono ellipsis">{settings.downloadDir ?? "Downloads"}</div>
          </div>
          <button className="btn-outline" onClick={onPickDownloadDir}>
            Change
          </button>
        </div>
      </div>
      <div className="settings-footer">
        <span className="mono">inphiso {version}</span>
        <span>GPL-3.0-or-later · bundles wimlib (GPLv3) and Geist (OFL)</span>
      </div>
      <div className="dialog-actions">
        <button className="btn-primary lg" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="setting">
      <div>
        <div className="setting-label">{label}</div>
        {hint && <div className="setting-hint">{hint}</div>}
      </div>
      <input type="checkbox" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}
