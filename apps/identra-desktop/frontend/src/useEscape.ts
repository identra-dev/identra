import { useEffect } from "react";

/// Close on Escape.
///
/// Extracted because it was true of the settings dialog and not of the two panels beside it, and a
/// key that closes one overlay but not the next one is worse than a key that closes nothing: the
/// user learns it works, then it silently does not. One implementation is how that stays true as
/// panels are added.
///
/// The listener is on `window` rather than the panel, because the panel is rarely what has focus.
/// A user reading a list has focus in a terminal or on the canvas behind it, and a handler bound to
/// the overlay would never hear the key that is supposed to dismiss it.
export function useEscape(onEscape: () => void) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onEscape();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onEscape]);
}
