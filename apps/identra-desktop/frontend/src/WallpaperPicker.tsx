// The wallpaper popover, opened by right-clicking the canvas background. Picking applies
// immediately: the choice is one field on the canvas and rides the debounced save, so there is
// nothing to confirm and no apply button to forget.
import { useCallback, useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  wallpaperAdd,
  wallpaperRemove,
  wallpapersList,
  type Wallpaper,
} from "./api";
import { BUILTINS, DEFAULT_WALLPAPER, SWATCHES } from "./wallpaper";

type Props = {
  current: Wallpaper;
  at: { x: number; y: number };
  onPick: (w: Wallpaper) => void;
  onClose: () => void;
};

// Kept below the popover's real height so clamping against the window edge never pushes it off
// the top instead.
const WIDTH = 296;
const HEIGHT = 320;

export default function WallpaperPicker({ current, at, onPick, onClose }: Props) {
  const [images, setImages] = useState<string[]>([]);
  // A failed add or remove lands here, inside the popover, because a dialog the user just drove
  // is the one place they are looking.
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    wallpapersList().then(setImages, () => setImages([]));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  const add = useCallback(async () => {
    try {
      const stored = await wallpaperAdd();
      if (stored === null) return; // cancelled, nothing to say
      refresh();
      onPick({ kind: "image", value: stored });
    } catch (e) {
      setError(String(e));
    }
  }, [onPick, refresh]);

  const remove = useCallback(
    async (path: string) => {
      if (!window.confirm("Remove this wallpaper from the library?")) return;
      try {
        await wallpaperRemove(path);
        refresh();
        // Removing the one this board is wearing goes back to the plain board now, rather than
        // leaving a background whose file is gone until the next reload notices.
        if (current.kind === "image" && current.value === path) {
          onPick(DEFAULT_WALLPAPER);
        }
      } catch (e) {
        setError(String(e));
      }
    },
    [current, onPick, refresh],
  );

  const isCurrent = (w: Wallpaper) =>
    current.kind === w.kind && current.value === w.value;

  // Clamped so a right-click near the window edge does not open a popover half off screen.
  const left = Math.min(at.x, window.innerWidth - WIDTH - 12);
  const top = Math.min(at.y, window.innerHeight - HEIGHT - 12);

  return (
    <>
      <div className="identra-wallpapers__backdrop" onMouseDown={onClose} />
      <div className="identra-wallpapers" style={{ left, top }}>
        <h4>Wallpaper</h4>
        {error !== null && (
          <p className="identra-wallpapers__error" role="alert">
            {error}
          </p>
        )}
        <div className="identra-wallpapers__grid">
          {BUILTINS.map((b) => (
            <button
              key={b.id}
              className="identra-wallpapers__tile"
              data-current={isCurrent({ kind: "yaru", value: b.id })}
              style={{ background: b.css }}
              title={b.label}
              onClick={() => onPick({ kind: "yaru", value: b.id })}
            />
          ))}
          {images.map((path) => (
            <span key={path} className="identra-wallpapers__cell">
              <button
                className="identra-wallpapers__tile"
                data-current={isCurrent({ kind: "image", value: path })}
                style={{
                  backgroundImage: `url("${convertFileSrc(path).replace(/"/g, "%22")}")`,
                }}
                title="Use this wallpaper"
                onClick={() => onPick({ kind: "image", value: path })}
              />
              <button
                className="identra-wallpapers__remove"
                title="Remove from the library"
                onClick={() => void remove(path)}
              >
                &times;
              </button>
            </span>
          ))}
          <button
            className="identra-wallpapers__tile identra-wallpapers__add"
            title="Add an image from disk"
            onClick={() => void add()}
          >
            +
          </button>
        </div>
        <div className="identra-wallpapers__swatches">
          {SWATCHES.map((hex) => (
            <button
              key={hex}
              className="identra-wallpapers__swatch"
              data-current={isCurrent({ kind: "color", value: hex })}
              style={{ background: hex }}
              title={hex}
              onClick={() => onPick({ kind: "color", value: hex })}
            />
          ))}
        </div>
      </div>
    </>
  );
}
