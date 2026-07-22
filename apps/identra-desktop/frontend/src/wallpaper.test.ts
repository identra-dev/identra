import { expect, test } from "bun:test";
import {
  backgroundCss,
  BUILTINS,
  DEFAULT_WALLPAPER,
  dotColor,
  needsScrim,
} from "./wallpaper";

const asUrl = (p: string) => `asset://${p}`;

test("every builtin id resolves to its own css and an unknown one falls back", () => {
  for (const b of BUILTINS) {
    expect(backgroundCss({ kind: "yaru", value: b.id }, asUrl)).toBe(b.css);
  }
  // A canvas from a newer build can name a background this build has never heard of. It has to
  // open as the plain board, because a hole where the canvas should be is a broken app.
  expect(backgroundCss({ kind: "yaru", value: "yaru-from-the-future" }, asUrl)).toBe(
    BUILTINS[0]!.css,
  );
});

test("a color is itself and an image keeps a floor under it", () => {
  expect(backgroundCss({ kind: "color", value: "#16161d" }, asUrl)).toBe("#16161d");
  const css = backgroundCss({ kind: "image", value: "/lib/abc.png" }, asUrl);
  expect(css).toContain('url("asset:///lib/abc.png")');
  // The base color under the image is what a missing file degrades to.
  expect(css).toContain("var(--bg)");
});

test("a quote in a filename cannot break out of the css url", () => {
  const css = backgroundCss({ kind: "image", value: '/lib/a"b.png' }, (p) => p);
  expect(css).not.toContain('"b');
  expect(css).toContain("%22");
});

test("only a user image needs the scrim, and only the plain board keeps grey dots", () => {
  expect(needsScrim({ kind: "image", value: "/lib/a.png" })).toBe(true);
  expect(needsScrim({ kind: "yaru", value: "yaru-aubergine" })).toBe(false);
  expect(needsScrim({ kind: "color", value: "#16161d" })).toBe(false);

  expect(dotColor(DEFAULT_WALLPAPER)).toBe("#3a3a3a");
  expect(dotColor({ kind: "yaru", value: "yaru-dusk" })).toBe("var(--wallpaper-dots)");
  expect(dotColor({ kind: "image", value: "/lib/a.png" })).toBe("var(--wallpaper-dots)");
});
