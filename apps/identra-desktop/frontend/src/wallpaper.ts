// What each wallpaper choice looks like, as CSS. Pure on purpose: the asset-URL conversion is
// passed in, so this file needs no Tauri runtime and the judgement in it can run under bun test.
import type { Wallpaper } from "./api";

export type Builtin = { id: string; label: string; css: string };

// The built-in set. Gradients rather than shipped image files: they cost nothing in the bundle,
// they are ours rather than someone's photography, and at canvas scale a quiet two-stop gradient
// reads as a wallpaper anyway. All of them stay dark enough that node chrome and white grid dots
// hold up on top.
export const BUILTINS: Builtin[] = [
  // The plain board, and the default. var(--bg) rather than a literal so it follows the app
  // background if that token ever changes.
  { id: "yaru-default", label: "Slate", css: "var(--bg)" },
  {
    id: "yaru-aubergine",
    label: "Aubergine",
    css: "linear-gradient(150deg, #231021, #4b1c44)",
  },
  {
    id: "yaru-ember",
    label: "Ember",
    css: "linear-gradient(150deg, #1d130e, #66290f)",
  },
  {
    id: "yaru-dusk",
    label: "Dusk",
    css: "linear-gradient(165deg, #14141b, #2b2b3d)",
  },
];

// The flat-color row, for people who want no picture at all. Curated dark values, which is why
// the color case needs no scrim: nothing here can fight the UI for contrast.
export const SWATCHES = ["#1d1d1d", "#16161d", "#1a2023", "#231a15", "#2a1420"];

export const DEFAULT_WALLPAPER: Wallpaper = {
  kind: "yaru",
  value: "yaru-default",
};

// A path can hold anything a filesystem allows, and this string lands inside a CSS url() in an
// inline style. Quoting plus escaping the quote is what keeps a strange filename a strange
// filename instead of a way out of the declaration.
const cssUrl = (u: string) => `url("${u.replace(/"/g, "%22")}")`;

/// The CSS background for a wallpaper choice. `imageUrl` converts a library path into something
/// the webview may load (convertFileSrc in the app, anything in a test).
export function backgroundCss(
  w: Wallpaper,
  imageUrl: (path: string) => string,
): string {
  switch (w.kind) {
    case "yaru":
      // An id this build does not know renders as the default rather than a hole. That is what
      // lets a canvas exported from a newer Identra still open here.
      return (
        BUILTINS.find((b) => b.id === w.value)?.css ?? BUILTINS[0]!.css
      );
    case "color":
      return w.value;
    case "image":
      // The plain background sits under the image, so a file that is gone (removed from the
      // library, or a canvas from another machine) degrades to the default board, not to a
      // broken one.
      return `${cssUrl(imageUrl(w.value))} center / cover no-repeat, var(--bg)`;
  }
}

/// Only a user image needs the scrim. The built-ins and the swatch row are curated dark values,
/// but an image is whatever the user had, and UI cannot sit on an arbitrary picture directly.
export const needsScrim = (w: Wallpaper): boolean => w.kind === "image";

/// The grid dot color for a wallpaper. On the plain board the dots keep their quiet grey; on
/// anything with its own character they flip to the wallpaper-dots token, white at low alpha,
/// which reads on every background the picker can produce.
export function dotColor(w: Wallpaper): string {
  return w.kind === "yaru" && w.value === "yaru-default"
    ? "#3a3a3a"
    : "var(--wallpaper-dots)";
}
