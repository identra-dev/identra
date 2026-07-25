// The settings popover. One machine-level choice today, laid out so the next one is a row, not a
// redesign. Everything per-workspace (title, wallpaper, the seat) lives elsewhere on purpose.
import { useEffect, useState } from "react";
import { settingsGet, settingsSet, type Settings } from "./api";
import { useEscape } from "./useEscape";

export default function SettingsPanel({ onClose }: { onClose: () => void }) {
  // Null until the engine answers. The panel renders nothing but its frame in that beat, rather
  // than a default that flickers to the real value.
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    settingsGet().then(setSettings, (e) => setError(String(e)));
  }, []);

  useEscape(onClose);

  // Optimistic, then honest: the checkbox moves at once, and a failed write puts it back and says
  // why. A toggle that silently did not stick is the settings version of a lost save. Shared by
  // every row so the second setting could not quietly skip the rollback the first one has.
  const apply = async (next: Settings) => {
    if (settings === null) return;
    const before = settings;
    setSettings(next);
    try {
      await settingsSet(next);
      setError(null);
    } catch (e) {
      setSettings(before);
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
          <>
            <label className="identra-settings__row">
              <input
                type="checkbox"
                checked={settings.autonomy === "bypass"}
                onChange={() =>
                  void apply({
                    ...settings,
                    autonomy: settings.autonomy === "bypass" ? "ask" : "bypass",
                  })
                }
              />
              <span>
                <span className="identra-settings__name">
                  Let agents work without asking
                </span>
                {/* Said plainly, including the part that is not comfortable. This is on by
                    default and it hands an agent the same reach the person at the keyboard
                    has, so the panel is the wrong place to be coy about it. */}
                <span className="identra-settings__hint">
                  On, every prompt each agent has a switch for is off:
                  approvals, sandboxing, folder trust, and consenting to
                  Identra's own context bus. Agents start working the moment
                  they open and connect to each other on their own. It also
                  means an agent can reach anything you can, so run it on work
                  you have committed. Off, every agent keeps its own prompts and
                  you answer them in the node. Applies to the next agent you
                  drop.
                </span>
              </span>
            </label>
            <label className="identra-settings__row">
              <input
                type="checkbox"
                checked={settings.embeddings}
                onChange={() =>
                  void apply({ ...settings, embeddings: !settings.embeddings })
                }
              />
              <span>
                <span className="identra-settings__name">
                  Recall by meaning
                </span>
                <span className="identra-settings__hint">
                  Uses a local model, fetched once (about 130MB). This is the
                  only thing Identra ever downloads. Off, recall matches on
                  words and nothing is fetched. Takes effect when Identra
                  restarts.
                </span>
              </span>
            </label>
          </>
        )}
      </div>
    </>
  );
}
