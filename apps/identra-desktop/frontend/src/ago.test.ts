import { test, expect } from "bun:test";
import { ago } from "./ago";

// A fixed "now" so this says something about the function rather than about when it ran.
const NOW = 1_800_000_000;
const at = (secondsBack: number) => ago(NOW - secondsBack, NOW);

test("recent facts read as recent, and the units switch where a person would switch them", () => {
  expect(at(0)).toBe("just now");
  expect(at(44)).toBe("just now");

  // Past the "just now" window it starts counting, and singular reads as singular.
  expect(at(60)).toBe("1 minute ago");
  expect(at(600)).toBe("10 minutes ago");
  expect(at(59 * 60)).toBe("59 minutes ago");

  expect(at(60 * 60)).toBe("1 hour ago");
  expect(at(5 * 60 * 60)).toBe("5 hours ago");

  // A day is "yesterday" rather than "1 day ago", which is how anyone would say it.
  expect(at(24 * 60 * 60)).toBe("yesterday");
  expect(at(3 * 24 * 60 * 60)).toBe("3 days ago");
});

test("a clock that drifted never puts a stored fact in the future", () => {
  // An agent whose clock runs a second ahead writes a fact stamped after the panel's own now.
  // Nothing stored can be from the future, so the honest reading is that it just happened.
  expect(ago(NOW + 1, NOW)).toBe("just now");
  expect(ago(NOW + 5000, NOW)).toBe("just now");
});
