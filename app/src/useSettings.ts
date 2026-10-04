import { useCallback, useState } from "react";
import { defaultSettings, type Settings } from "./types";

const KEY = "inphiso.settings";

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? { ...defaultSettings, ...JSON.parse(raw) } : defaultSettings;
  } catch {
    return defaultSettings;
  }
}

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(load);
  const update = useCallback((patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      try {
        localStorage.setItem(KEY, JSON.stringify(next));
      } catch {
        // Storage unavailable: settings just won't persist.
      }
      return next;
    });
  }, []);
  return [settings, update] as const;
}
