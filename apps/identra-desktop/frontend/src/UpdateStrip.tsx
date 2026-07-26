// The auto-update surface: one quiet strip that only exists when a newer Identra is really
// available. Checking happens once per launch; installing happens when the user says so, never
// on its own, because a canvas of running agents is the wrong thing to restart out from under
// someone.
import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

type State =
  | { at: "idle" }
  | { at: "offered"; update: Update }
  | { at: "installing" }
  | { at: "done" }
  | { at: "failed"; why: string };

// The headline of a release note, for a strip one line tall. Release bodies are markdown with a
// summary line and then bullets, so the first non-empty line is the sentence a person would have
// written to answer "what is this one".
function firstLine(body: string): string {
  const line = body.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
  return line.length > 90 ? `${line.slice(0, 89)}…` : line;
}

export default function UpdateStrip() {
  const [state, setState] = useState<State>({ at: "idle" });

  useEffect(() => {
    let dropped = false;
    // A failed check is silence, not an error strip. Being offline, or a dev build with no
    // release behind it, is the ordinary case; nagging about it would train people to ignore
    // the strip that matters.
    check().then(
      (update) => {
        if (!dropped && update !== null) setState({ at: "offered", update });
      },
      () => {},
    );
    return () => {
      dropped = true;
    };
  }, []);

  if (state.at === "idle") return null;

  const install = async () => {
    if (state.at !== "offered") return;
    const update = state.update;
    setState({ at: "installing" });
    try {
      await update.downloadAndInstall();
      // No forced relaunch: the user closes Identra when their agents are done, and the new
      // build is simply what opens next time.
      setState({ at: "done" });
    } catch (e) {
      setState({ at: "failed", why: String(e) });
    }
  };

  return (
    <div className="identra-update" role="status">
      {state.at === "offered" && (
        <>
          <span>Identra {state.update.version} is available.</span>
          {/* What changed, when the release said. Asking someone to install an unnamed change to
              the thing that runs agents against their code is asking for trust with nothing to
              base it on, and the honest answer to "should I?" is the release notes. They ride
              along on the update manifest already, so this costs no fetch: it was being thrown
              away. Truncated because this is a strip, not a changelog, and the full text is on
              the release page. */}
          {state.update.body !== undefined && state.update.body !== "" && (
            <span className="identra-update__notes" title={state.update.body}>
              {firstLine(state.update.body)}
            </span>
          )}
          <button className="identra-update__go" onClick={() => void install()}>
            Install
          </button>
          <button
            className="identra-update__later"
            onClick={() => setState({ at: "idle" })}
          >
            Later
          </button>
        </>
      )}
      {state.at === "installing" && <span>Downloading the update...</span>}
      {state.at === "done" && (
        <span>Updated. The new Identra opens the next time you start it.</span>
      )}
      {state.at === "failed" && (
        <span>The update did not install: {state.why}</span>
      )}
    </div>
  );
}
