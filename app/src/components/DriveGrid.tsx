import { formatBytes } from "../format";
import type { Drive } from "../types";
import { Check } from "./icons";

interface Props {
  drives: Drive[];
  loaded: boolean;
  pickedId: string | null;
  /** No image yet: cards are shown dimmed and can't be picked. */
  inactive: boolean;
  fits: (d: Drive) => boolean;
  onPick: (id: string) => void;
}

export function DriveGrid({ drives, loaded, pickedId, inactive, fits, onPick }: Props) {
  if (!loaded) return <div className="drive-empty">Looking for drives…</div>;
  if (drives.length === 0) {
    return (
      <div className="drive-empty">
        No removable drives found. Plug in a USB stick or SD card.
      </div>
    );
  }
  return (
    <div className="drive-grid">
      {drives.map((d) => {
        const tooSmall = !inactive && !fits(d);
        const disabled = inactive || tooSmall;
        const on = !inactive && d.id === pickedId;
        const meta = [
          formatBytes(d.size),
          tooSmall ? "too small" : d.bus,
          !tooSmall && !inactive ? d.id : null,
        ]
          .filter(Boolean)
          .join(" · ");
        return (
          <button
            key={d.id}
            className={["drive-card", on && "on", inactive && "inactive", tooSmall && "too-small"]
              .filter(Boolean)
              .join(" ")}
            disabled={disabled}
            onClick={() => onPick(d.id)}
            aria-pressed={on}
          >
            <div className="drive-top">
              <div className="drive-name">
                {d.name}
                {!d.removable && <span className="tag">internal</span>}
              </div>
              {!inactive && <div className={on ? "dot on" : "dot"}>{on && <Check size={12} weight={3.4} />}</div>}
            </div>
            <div className="drive-meta">{meta}</div>
          </button>
        );
      })}
    </div>
  );
}
