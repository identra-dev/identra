// Finding the dev server's address in its own output. Pure, because the judgement (what counts
// as the preview URL) is the part worth a test, and it must not care which terminal the bytes
// came through.

// Color codes wrap the URL in every dev server's output; strip them or the match starts inside
// an escape sequence.
const ANSI = /\x1b\[[0-9;]*[A-Za-z]/g;

// Loopback only, on purpose. A dev server prints its Network address too, and previewing over
// the LAN is not what the one-click flow means; the local one is always there and always right.
const LOCAL = /https?:\/\/(?:localhost|127\.0\.0\.1):\d+[^\s"'\x1b]*/;

/// The first local URL in `text`, or null. Trailing slash and punctuation are trimmed so vite's
/// `http://localhost:5173/` and a sentence ending `http://localhost:3000.` both come out clean.
export function findLocalUrl(text: string): string | null {
  const hit = LOCAL.exec(text.replace(ANSI, ""));
  if (hit === null) return null;
  return hit[0].replace(/[/.,)\]]+$/, "");
}

/// A rolling window over a chunked stream, so a URL split across two output chunks still matches.
/// The window is plenty for any banner line and stays small however long the server runs.
export function appendTail(tail: string, chunk: string, max = 4096): string {
  const joined = tail + chunk;
  return joined.length > max ? joined.slice(joined.length - max) : joined;
}
