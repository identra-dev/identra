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
