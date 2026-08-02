// Starting the process behind a pane, and saying what happened when it does not start.
//
// This used to live inside the canvas node, which meant the only thing that could launch an agent
// was a component that also drew a draggable box. The shell has panes instead, and a pane is a view
// that comes and goes over a process that does not, so the launching had to stop being part of any
// one view's render.
//
// Every failure here returns a line to print in the pane's own terminal rather than throwing. The
// user is looking at a black rectangle where an agent should be; a line in that rectangle is the
// message, and an exception thrown past it is a blank rectangle plus a console entry nobody reads.
import { agentsByKind, devCommand, terminalStart } from "./api";

const red = (s: string) => `\r\n\x1b[31m${s}\x1b[0m`;

/// Start `nodeId`'s process. Returns the line to print, or null when it started cleanly.
///
/// `kind` is the agent id (codex, claude, …) except for "dev", which runs the project's own dev
/// command instead of a CLI from the registry.
export async function startNode(
  nodeId: string,
  kind: string,
  cwd: string | null,
  rows: number,
  cols: number,
): Promise<string | null> {
  if (kind === "dev") {
    const cmd = await devCommand();
    if (cmd === null || cmd.length === 0) {
      return `${red("This project does not declare a dev command")} in package.json, a justfile, or a Makefile.\r\n`;
    }
    try {
      await terminalStart(nodeId, kind, cmd[0]!, cmd.slice(1), cwd, rows, cols);
      return null;
    } catch (e) {
      return `${red("The dev server didn't start:")} ${String(e)}\r\n`;
    }
  }

  const registry = await agentsByKind();
  const agent = registry.get(kind);
  if (!agent || !agent.available) {
    // Say what to do with what is actually on this machine.
    //
    // This used to tell people to run `just doctor`. That command lives in Identra's own repo, so
    // someone who installed the app had no repo, no justfile and most likely no `just` — the first
    // error the product ever showed them offered a fix they could not carry out. An error naming an
    // impossible next step is worse than one naming none, because it costs a search before it comes
    // to nothing. What is useful here is the machine's own answer.
    const ready = [...registry.values()]
      .filter((a) => a.available)
      .map((a) => a.name);
    return (
      `${red(`${kind} isn't installed`)} or not on your PATH.\r\n` +
      (ready.length > 0
        ? `Already installed here: ${ready.join(", ")}. Close this tab and open one of those instead.\r\n`
        : "Install a coding agent, then reopen this tab. Identra runs the CLI on your machine; it does not ship one.\r\n")
    );
  }

  try {
    await terminalStart(nodeId, kind, agent.cmd, agent.args, cwd, rows, cols);
    return null;
  } catch (e) {
    return `${red(`${agent.name} didn't start:`)} ${String(e)}\r\n`;
  }
}
