import { expect, test } from "bun:test";
import { appendTail, findLocalUrl } from "./devurl";

test("real banner shapes give up their url", () => {
  // vite, colors and all
  expect(
    findLocalUrl("  \x1b[32m➜\x1b[39m  Local:   \x1b[36mhttp://localhost:5173/\x1b[39m"),
  ).toBe("http://localhost:5173");
  // next
  expect(findLocalUrl("   - Local:        http://localhost:3000")).toBe(
    "http://localhost:3000",
  );
  // a plain server on the loopback ip, mid-sentence
  expect(findLocalUrl("Serving at http://127.0.0.1:8000, press q to quit")).toBe(
    "http://127.0.0.1:8000",
  );
  // a path survives, the trailing punctuation does not
  expect(findLocalUrl("open http://localhost:4321/docs/.")).toBe(
    "http://localhost:4321/docs",
  );
});

test("what is not the local preview is not matched", () => {
  // the LAN address a dev server prints next to the local one
  expect(findLocalUrl("Network: http://192.168.1.4:5173/")).toBe(null);
  // someone's docs link in a build warning
  expect(findLocalUrl("see https://vitejs.dev/config/ for details")).toBe(null);
  expect(findLocalUrl("no url here")).toBe(null);
});

test("a url split across two chunks is still found in the tail", () => {
  let tail = "";
  tail = appendTail(tail, "  Local:   http://loc");
  expect(findLocalUrl(tail)).toBe(null);
  tail = appendTail(tail, "alhost:5173/\n");
  expect(findLocalUrl(tail)).toBe("http://localhost:5173");
  // and the window does not grow without bound under a chatty server
  for (let i = 0; i < 100; i++) tail = appendTail(tail, "x".repeat(500));
  expect(tail.length).toBeLessThanOrEqual(4096);
});
