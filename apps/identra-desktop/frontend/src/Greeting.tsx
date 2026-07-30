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
import { useCallback, useEffect, useRef, useState } from "react";
import { settingsGet, settingsSet, type Settings } from "./api";
import { useEscape } from "./useEscape";

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

  // "Not now", as opposed to "no". Escape and a click outside both land here.
  //
  // This dialog was the one overlay in the app with no way out but its own two buttons: no Escape,
  // no click-away, on the very first screen a new install shows. Every other overlay closes on
  // Escape, so the key someone reflexively presses to dismiss a box did nothing exactly once, on
  // first run, which is the worst possible place to teach someone the app ignores them.
  //
  // Deliberately does NOT write settings. An empty name is a real, remembered answer meaning "do
  // not greet me", and that is what "Rather not" is for — a permanent choice deserves a deliberate
  // click, never a keystroke someone pressed to make a box go away. So dismissing leaves `name` as
  // `None` and the ask comes back next launch. Being asked again is the honest cost of not
  // answering; silently deciding on their behalf is not.
  //
  // The functional update is what keeps this correct without a dependency on `phase`: it only acts
  // while the question is actually on screen, so the window-level Escape listener cannot turn a
  // fading hello, or a quiet canvas, into a state change.
  const dismiss = useCallback(() => {
    setPhase((cur) => (cur.at === "asking" ? { at: "quiet" } : cur));
  }, []);
  useEscape(dismiss);

  if (phase.at === "loading" || phase.at === "quiet") return null;

  if (phase.at === "greeting") {
    // The whole window, blurred behind one line. It is the first second of the app and there is
    // nothing to do in it, so it can afford to be a moment rather than a notification in a corner.
    // Nothing is clickable and nothing waits: it fades on its own, and the blur goes with it.
    return (
      <div className="identra-greet" role="status">
        <p className="identra-greet__text">{phase.text}</p>
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
    <div
      className="identra-ask__scrim"
      // Same as the name-workspace dialog, and for the same reason: a one-field question with a way
      // out does not need to trap anybody. Clicking away is "not now", never "no".
      onMouseDown={dismiss}
    >
      <div
        className="identra-ask"
        role="dialog"
        aria-label="What to call you"
        // Without this, every click that lands on the card bubbles to the scrim and dismisses the
        // question the user was in the middle of answering — including the click that put the
        // cursor in the text field.
        onMouseDown={(e) => e.stopPropagation()}
      >
        <p className="identra-ask__lead">What should I call you?</p>
        <p className="identra-ask__sub">
          Identra says hello with it when it opens. It stays on this machine.
        </p>
        <form
          className="identra-ask__row"
          onSubmit={(e) => {
            e.preventDefault();
            void save(typed.trim());
          }}
        >
          <input
            className="identra-ask__input"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            placeholder={suggestion.current}
            aria-label="What to call you"
            maxLength={40}
            autoFocus
          />
          <button
            className="identra-ask__go"
            type="submit"
            disabled={typed.trim() === ""}
          >
            That&apos;s me
          </button>
        </form>
        {/* The way out has to be as easy as the way through, or this stops being a nicety and
            starts being a form. Declining is remembered, so it is asked once and never again. */}
        <button
          className="identra-ask__skip"
          type="button"
          onClick={() => void save("")}
        >
          Rather not
        </button>
      </div>
    </div>
  );
}
