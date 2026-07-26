import { expect, test } from "bun:test";
import { greetingFor } from "./Greeting";

// The one bit of logic here worth pinning. "Good evening" at 3pm is the tell that nobody is
// actually there, and the whole point of greeting someone by name is that somebody is.
test("the greeting matches the clock and uses the name as typed", () => {
  expect(greetingFor("Captain Segfault", 9)).toBe("Morning, Captain Segfault.");
  expect(greetingFor("Captain Segfault", 14)).toBe(
    "Afternoon, Captain Segfault.",
  );
  expect(greetingFor("Captain Segfault", 20)).toBe(
    "Evening, Captain Segfault.",
  );

  // Someone working at 2am is not having a morning, and saying so is the difference between a
  // greeting that reads as written for a person and one that reads as generated.
  expect(greetingFor("Doctor Rebase", 2)).toBe("Still up, Doctor Rebase.");

  // Boundaries, because an off-by-one here shows up as the app calling 11:59am an afternoon.
  expect(greetingFor("x", 5)).toBe("Morning, x.");
  expect(greetingFor("x", 11)).toBe("Morning, x.");
  expect(greetingFor("x", 12)).toBe("Afternoon, x.");
  expect(greetingFor("x", 17)).toBe("Afternoon, x.");
  expect(greetingFor("x", 18)).toBe("Evening, x.");

  // Whatever someone typed comes back as they typed it. Title-casing or trimming a name people
  // chose to be daft with is the app deciding it knows better.
  expect(greetingFor("xX_void*_Xx", 9)).toBe("Morning, xX_void*_Xx.");
});
