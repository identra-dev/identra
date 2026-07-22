// The settings popover. One machine-level choice today, laid out so the next one is a row, not a
// redesign. Everything per-workspace (title, wallpaper, the seat) lives elsewhere on purpose.
import { useEffect, useState } from "react";
import { settingsGet, settingsSet, type Settings } from "./api";

export default function SettingsPanel({ onClose }: { onClose: () => void }) {
  // Null until the engine answers. The panel renders nothing but its frame in that beat, rather
  // than a default that flickers to the real value.
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    settingsGet().then(setSettings, (e) => setError(String(e)));
  }, []);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  const toggleEmbeddings = async () => {
    if (settings === null) return;
    const next = { ...settings, embeddings: !settings.embeddings };
    // Optimistic, then honest: the checkbox moves at once, and a failed write puts it back and
    // says why. A toggle that silently did not stick is the settings version of a lost save.
    setSettings(next);
    try {
      await settingsSet(next);
      setError(null);
    } catch (e) {
      setSettings(settings);
      setError(String(e));
    }
  };

  return (
    <>
      <div className="identra-settings__backdrop" onMouseDown={onClose} />
      <div className="identra-settings" role="dialog" aria-label="Settings">
        <h4>Settings</h4>
        {error !== null && (
          <p className="identra-settings__error" role="alert">
            {error}
          </p>
        )}
        {settings !== null && (
          <label className="identra-settings__row">
            <input
              type="checkbox"
              checked={settings.embeddings}
              onChange={() => void toggleEmbeddings()}
            />
            <span>
              <span className="identra-settings__name">Recall by meaning</span>
              <span className="identra-settings__hint">
                Uses a local model, fetched once (about 130MB). This is the only
                thing Identra ever downloads. Off, recall matches on words and
                nothing is fetched. Takes effect when Identra restarts.
              </span>
            </span>
          </label>
        )}
      </div>
    </>
  );
}
