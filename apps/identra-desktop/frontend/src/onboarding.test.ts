import { test, expect } from "bun:test";
import { noAgentsInstalled, type AgentInfo } from "./api";

const agent = (id: string, available: boolean): AgentInfo => ({
  id,
  name: id,
  path: "",
  available,
  logged_in: false,
  cmd: id,
  args: [],
});

test("the install panel shows only once detection ran and every agent is missing", () => {
  // Empty means detection has not answered yet. Showing the panel here would flash it on every
  // launch before the first probe resolves, so it must stay hidden.
  expect(noAgentsInstalled([])).toBe(false);
  // Every known agent missing is the real first-run case the panel is for.
  expect(
    noAgentsInstalled([agent("codex", false), agent("claude", false)]),
  ).toBe(true);
  // One installed hides it, even if that agent is not signed in yet: there is now something to run.
  expect(
    noAgentsInstalled([agent("codex", true), agent("claude", false)]),
  ).toBe(false);
});
