// Who Identra is talking to.
//
// A tool you open every morning either says hello to somebody or to nobody, and the second one is
// what most software does. This is the small version of the first: ask once what you would like to
// be called, then use it. It is not a username and it is not an account — nothing authenticates
// against it, nothing is keyed by it, and it never leaves the settings file on this machine.
//
// The greeting shows itself and then gets out of the way. A permanent name badge is decoration you
// stop seeing by the third launch; a line that appears when you open the app and fades is the part
// that actually lands, because it only happens at the moment you arrive.
import { useEffect, useRef, useState } from "react";
import { settingsGet, settingsSet, type Settings } from "./api";

// How long the hello stays before it fades. Long enough to read at a glance and be sure it was
// meant for you, short enough that it is gone before you have finished deciding what to type.
const GREETING_MS = 4200;

// What goes in the box before anyone types. Suggestions rather than instructions: the ask is
// friendlier when the examples make it obvious that nothing here is being taken seriously.
const SUGGESTIONS = [
  "Captain Segfault",
  "The Merge Conflict",
  "Rear Admiral Nullpointer",
  "Doctor Rebase",
  "Chief Yak Shaver",
];

// Time of day, because "good evening" at 3pm is the tell that nobody is really there. The bands are
// generous on purpose: this is a greeting, not a clock.
function timeOfDay(hour: number): string {
  if (hour < 5) return "Still up";
  if (hour < 12) return "Morning";
  if (hour < 18) return "Afternoon";
  return "Evening";
}

export function greetingFor(name: string, hour: number): string {
  return `${timeOfDay(hour)}, ${name}.`;
}

type Phase =
  // Settings not read yet. Renders nothing: a prompt that flashes up and vanishes because the
  // answer was already on disk is worse than a beat of nothing.
  | { at: "loading" }
  | { at: "asking"; settings: Settings }
  | { at: "greeting"; text: string }
  | { at: "quiet" };

export default function Greeting() {
  const [phase, setPhase] = useState<Phase>({ at: "loading" });
  const [typed, setTyped] = useState("");
  // One suggestion per launch rather than per keystroke, so the placeholder does not shuffle
  // underneath someone who is reading it.
  const suggestion = useRef(
    SUGGESTIONS[Math.floor(Math.random() * SUGGESTIONS.length)],
  );

  useEffect(() => {
    let dropped = false;
    settingsGet().then(
      (settings) => {
        if (dropped) return;
        // null is never asked. Empty is asked and declined, and that answer is kept: a friendly
        // touch that re-asks every launch is a nag.
        if (settings.name === null) {
          setPhase({ at: "asking", settings });
        } else if (settings.name !== "") {
          setPhase({
            at: "greeting",
            text: greetingFor(settings.name, new Date().getHours()),
          });
        } else {
          setPhase({ at: "quiet" });
        }
      },
      // Settings that will not load is a real problem and this is not the place to raise it: the
      // canvas is what someone came for, and a hello is not worth an error banner in front of it.
      () => {
        if (!dropped) setPhase({ at: "quiet" });
      },
    );
    return () => {
      dropped = true;
    };
  }, []);

  // The fade. Tied to entering the greeting phase rather than to mount, so a name saved just now
  // gets the same few seconds as one read off disk.
  useEffect(() => {
    if (phase.at !== "greeting") return;
    const timer = window.setTimeout(
      () => setPhase({ at: "quiet" }),
      GREETING_MS,
    );
    return () => window.clearTimeout(timer);
  }, [phase.at]);

  if (phase.at === "loading" || phase.at === "quiet") return null;

  if (phase.at === "greeting") {
    return (
      <div className="identra-greeting" role="status">
        {phase.text}
      </div>
    );
  }

  const save = async (name: string) => {
    const settings = phase.settings;
    // Optimistic: the answer is on screen before the write lands. A failed write means the app
    // asks again next launch, which is a far smaller cost than making someone wait on a disk to
    // find out whether their own name took.
    setPhase(
      name === ""
        ? { at: "quiet" }
        : { at: "greeting", text: greetingFor(name, new Date().getHours()) },
    );
    try {
      await settingsSet({ ...settings, name });
    } catch {
      /* asked again next time, which is the right failure for this */
    }
  };

  return (
    <div className="identra-hello" role="dialog" aria-label="What to call you">
      <p className="identra-hello__lead">What should I call you?</p>
      <form
        className="identra-hello__row"
        onSubmit={(e) => {
          e.preventDefault();
          void save(typed.trim());
        }}
      >
        <input
          className="identra-hello__input"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={suggestion.current}
          aria-label="What to call you"
          maxLength={40}
          autoFocus
        />
        <button
          className="identra-hello__go"
          type="submit"
          disabled={typed.trim() === ""}
        >
          That&apos;s me
        </button>
      </form>
      {/* The way out has to be as easy as the way through, or this stops being a nicety and starts
          being a form. Declining is remembered, so it is asked once and never again. */}
      <button
        className="identra-hello__skip"
        type="button"
        onClick={() => void save("")}
      >
        Rather not
      </button>
    </div>
  );
}
