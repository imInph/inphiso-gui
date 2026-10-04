// Formatting matches the mockups: decimal units, one decimal below 100, no trailing ".0".

export function formatBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) {
    n /= 1000;
    i++;
  }
  if (i === 0) return `${n} B`;
  const s = n.toFixed(n >= 100 ? 0 : 1).replace(/\.0$/, "");
  return `${s} ${units[i]}`;
}

/** "3.8 of 6.1 GB": shares the unit of the total. */
export function formatBytesOf(done: number, total: number): string {
  const t = formatBytes(total);
  const unit = t.split(" ")[1];
  const scale = { B: 1, KB: 1e3, MB: 1e6, GB: 1e9, TB: 1e12 }[unit] ?? 1;
  return `${(done / scale).toFixed(1).replace(/\.0$/, "")} of ${t}`;
}

export function formatSpeed(bytesPerSec: number): string {
  return `${(bytesPerSec / 1e6).toFixed(1)} MB/s`;
}

export function formatEta(secs: number | null): string {
  if (secs == null) return "working it out";
  if (secs < 45) return "less than a minute";
  const mins = Math.round(secs / 60);
  if (mins < 60) return `about ${mins} min`;
  const h = Math.floor(mins / 60);
  return `about ${h} h ${mins % 60} min`;
}

export function formatDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  if (m === 0) return `${s} s`;
  if (m < 60) return `${m} min ${s} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function shortHash(hash: string): string {
  return hash.length > 12 ? `${hash.slice(0, 8)}…${hash.slice(-4)}` : hash;
}

/** "ubuntu-24.04.1-desktop-amd64.iso" → "ubuntu-24.04.1" for the compact header. */
export function shortImageName(name: string): string {
  const base = name.replace(/\.(iso|img|raw|dd|usb)(\.(xz|gz|bz2|zst))?$/i, "");
  const m = base.match(/^([a-z]+[-_]?[\d.]+)/i);
  return m ? m[1] : base;
}
