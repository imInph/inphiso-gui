import type { ReactNode } from "react";
import { formatBytes, formatDuration, shortHash } from "../format";
import type { ChecksumState, Drive, ImageInfo } from "../types";
import { Alert, Check, Cross } from "./icons";

export function Done({
  image,
  drive,
  checksum,
  elapsedMs,
  verified,
  ejected,
  ejecting,
  onAnother,
  onEject,
}: {
  image: ImageInfo;
  drive: Drive | undefined;
  checksum: ChecksumState;
  elapsedMs: number;
  verified: boolean;
  ejected: boolean;
  ejecting: boolean;
  onAnother: () => void;
  onEject: () => void;
}) {
  return (
    <div className="outcome">
      <Badge tone="ok">
        <Check size={24} weight={2.8} />
      </Badge>
      <div className="outcome-titles">
        <div className="outcome-title">Ready to boot</div>
        <div className="outcome-sub">
          {verified ? "Written and verified." : "Written."}{" "}
          {ejected ? "The drive has been ejected." : "It's safe to eject."}
        </div>
      </div>
      <div className="summary">
        <Row label="Image" value={image.name} mono />
        <Row label="Drive" value={drive ? `${drive.name} · ${formatBytes(drive.size)}` : "—"} />
        <Row label="Checksum" value={<ChecksumValue state={checksum} />} />
        <Row label="Took" value={formatDuration(elapsedMs)} mono />
      </div>
      <div className="outcome-actions">
        <button className="btn-outline lg" onClick={onAnother}>
          Flash another
        </button>
        <button className="btn-primary lg" onClick={onEject} disabled={ejected || ejecting}>
          {ejected ? "Ejected" : ejecting ? "Ejecting…" : "Eject drive"}
        </button>
      </div>
    </div>
  );
}

function ChecksumValue({ state }: { state: ChecksumState }) {
  switch (state.status) {
    case "match":
      return <span className="mono ok">{shortHash(state.actual)} ✓</span>;
    case "mismatch":
      return <span className="mono bad">{shortHash(state.actual)} ✗ mismatch</span>;
    case "unverified":
      return <span className="mono muted">{shortHash(state.actual)} · not compared</span>;
    default:
      return <span className="muted">—</span>;
  }
}

export function Failed({
  message,
  onBack,
  onRetry,
}: {
  message: string;
  onBack: () => void;
  onRetry: () => void;
}) {
  return (
    <div className="outcome">
      <Badge tone="bad">
        <Cross size={24} weight={2.8} />
      </Badge>
      <div className="outcome-titles">
        <div className="outcome-title">Flash failed</div>
        <div className="outcome-sub error-text">{message}</div>
        <div className="outcome-note">The drive may be partly written. Flash it again before using it.</div>
      </div>
      <div className="outcome-actions">
        <button className="btn-outline lg" onClick={onBack}>
          Back
        </button>
        <button className="btn-primary lg" onClick={onRetry}>
          Try again
        </button>
      </div>
    </div>
  );
}

export function Cancelled({ onBack }: { onBack: () => void }) {
  return (
    <div className="outcome">
      <Badge tone="warn">
        <Alert size={24} weight={2.4} />
      </Badge>
      <div className="outcome-titles">
        <div className="outcome-title">Cancelled</div>
        <div className="outcome-sub">
          The drive was only partly written and won't boot. Flash it again or reformat it.
        </div>
      </div>
      <div className="outcome-actions">
        <button className="btn-primary lg" onClick={onBack}>
          Back
        </button>
      </div>
    </div>
  );
}

function Badge({ tone, children }: { tone: "ok" | "bad" | "warn"; children: ReactNode }) {
  return (
    <div className={`badge badge-${tone}`}>
      <div className="badge-inner">{children}</div>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="summary-row">
      <span className="muted">{label}</span>
      <span className={mono ? "mono" : undefined}>{value}</span>
    </div>
  );
}
