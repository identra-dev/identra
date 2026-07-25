// When the project learned something, in the words a person scanning a list would use.
//
// Pure and separate from the panel because the judgement here is the part worth a test: what counts
// as "just now", where the units switch, and that it never says "in 3 minutes" for a clock that
// drifted a second. The panel does the drawing.

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/// A unix-seconds timestamp as a short relative phrase, given `now` in the same units.
///
/// `now` is a parameter rather than read from the clock inside, so this is deterministic and a test
/// does not have to freeze time to say anything about it.
///
/// The scale is deliberately coarse and stops at days. A memory list is scanned for "is this from
/// this session or from last week", and the difference between 4 and 5 minutes never changes what
/// anyone does about a fact. Precision past the decision is noise dressed as detail.
export function ago(createdAt: number, now: number): string {
  // A clock that stepped backwards, or a fact written by an agent whose clock is a second ahead,
  // must not render as the future. Nothing about a stored fact can be from the future, so the
  // honest reading of a negative gap is "this just happened".
  const seconds = Math.max(0, now - createdAt);

  if (seconds < 45) return "just now";
  if (seconds < HOUR) {
    const m = Math.round(seconds / MINUTE);
    return m === 1 ? "1 minute ago" : `${m} minutes ago`;
  }
  if (seconds < DAY) {
    const h = Math.round(seconds / HOUR);
    return h === 1 ? "1 hour ago" : `${h} hours ago`;
  }
  const d = Math.round(seconds / DAY);
  if (d === 1) return "yesterday";
  return `${d} days ago`;
}
