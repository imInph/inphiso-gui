import { formatBytes, formatBytesOf, formatEta, formatSpeed } from "../format";
import type { Progress } from "../types";

interface Props {
  progress: Progress | null;
  speeds: Array<[number, number]>;
  verify: boolean;
}

// The write segment takes 3/4 of the bar when verification follows, as in the mockup.
const WRITE_SHARE = 0.75;

export function Writing({ progress, speeds, verify }: Props) {
  const phase = progress?.phase ?? "write";
  const frac = progress?.total ? Math.min(progress.bytes / progress.total, 1) : 0;
  const writeFrac = phase === "write" ? frac : 1;
  const verifyFrac = phase === "verify" ? frac : 0;
  const overall = verify ? writeFrac * WRITE_SHARE + verifyFrac * (1 - WRITE_SHARE) : writeFrac;
  const pct = Math.floor(overall * 100);
  const peak = progress?.peak ?? 0;

  return (
    <div className="writing-card">
      <div className="writing-head">
        <div className="writing-titles">
          <div className="writing-title">{phase === "verify" ? "Verifying" : "Writing image"}</div>
          <div className="writing-sub">
            {phase === "verify"
              ? "Reading the drive back to check every byte."
              : verify
                ? "Don't unplug the drive. Verification starts automatically."
                : "Don't unplug the drive."}
          </div>
        </div>
        <div className="writing-pct">
          {progress?.total ? pct : "–"}
          <span>%</span>
        </div>
      </div>

      <div className="phase-bar">
        <div className="phase-track">
          <div className={verify ? "seg seg-write" : "seg seg-solo"}>
            <div className="seg-fill" style={{ width: `${writeFrac * 100}%` }} />
          </div>
          {verify && (
            <div className="seg seg-verify">
              <div className="seg-fill" style={{ width: `${verifyFrac * 100}%` }} />
            </div>
          )}
        </div>
        <div className="phase-labels">
          <div className={verify ? "seg-write" : "seg-solo"} data-active={phase === "write"}>
            Write
          </div>
          {verify && (
            <div className="seg-verify" data-active={phase === "verify"}>
              Verify
            </div>
          )}
        </div>
      </div>

      <div className="throughput">
        <div className="throughput-head">
          <span>Throughput</span>
          <span className="mono">{peak > 0 ? `peak ${formatSpeed(peak)}` : ""}</span>
        </div>
        <SpeedChart points={speeds} peak={peak} />
      </div>

      <div className="stats">
        <Stat label="Speed" value={progress ? formatSpeed(progress.speed) : "–"} />
        <Stat
          label={phase === "verify" ? "Verified" : "Written"}
          value={
            progress
              ? progress.total
                ? formatBytesOf(progress.bytes, progress.total)
                : formatBytes(progress.bytes)
              : "–"
          }
        />
        <Stat label="Time left" value={progress ? formatEta(progress.etaSecs) : "–"} />
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
    </div>
  );
}

const W = 840;
const H = 120;

function SpeedChart({ points, peak }: { points: Array<[number, number]>; peak: number }) {
  // Leave headroom above the peak so the line never touches the top edge.
  const max = peak > 0 ? peak * 1.3 : 1;
  const xy = points.map(([x, s]) => `${(x * W).toFixed(1)},${(H - (s / max) * H).toFixed(1)}`);
  const firstX = points.length ? points[0][0] * W : 0;
  const lastX = points.length ? points[points.length - 1][0] * W : 0;
  return (
    <svg className="chart" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      {points.length > 1 && (
        <>
          <polygon
            points={`${firstX.toFixed(1)},${H} ${xy.join(" ")} ${lastX.toFixed(1)},${H}`}
            className="chart-area"
          />
          <polyline points={xy.join(" ")} className="chart-line" />
        </>
      )}
      <line x1="0" x2={W} y1={H - 1} y2={H - 1} className="chart-base" />
    </svg>
  );
}
