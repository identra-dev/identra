// Every connection in this workspace, who granted it, and how to take it back.
//
// A connection is not decoration. `canvas.rs` states it directly: two nodes share context only when
// an edge joins them, and `identra-mcp` enforces it in three places — `get_peer_context` refuses
// without one, `list_peers` reads this exact slice, and `land_work` refuses to land the work of a
// helper you are not connected to. Drawing a wire was how a person granted that. Nothing draws a
// wire any more, so this is the grant, and this is the only place it can be seen or withdrawn.
//
// Two rules this panel is built under, both of them the reason it exists rather than details of it:
//
// It lists every connection whatever made it. `connect_nodes` is a bus command any agent on the bus
// can issue, gated only by the lock on either end, so agents can grant themselves peer access. The
// canvas made that tolerable by accident: a line appeared in front of you, unasked. A panel that
// showed only the connections you made would look like a complete list and would not be one, which
// is worse than the canvas was, not better.
//
// It never guesses. A connection saved before Identra recorded who made it says so, in those words.
// The one thing this panel must never do is answer "you" about a grant it cannot account for.
import { useEscape } from "./useEscape";
import { AgentIcon } from "./icons";
import type { CanvasNode, Edge } from "./api";

type Props = {
  nodes: CanvasNode[];
  edges: Edge[];
  onRevoke: (edgeId: string) => void;
  onClose: () => void;
};

const SAID: Record<Edge["by"], string> = {
  you: "You connected these",
  agent: "An agent connected these itself",
  unknown: "Connected before Identra recorded who by",
};

export default function ConnectionsPanel({
  nodes,
  edges,
  onRevoke,
  onClose,
}: Props) {
  useEscape(onClose);
  const name = (id: string) => {
    const n = nodes.find((x) => x.id === id);
    return {
      title: n?.title || n?.kind || "a closed tab",
      kind: n?.kind ?? "",
    };
  };

  return (
    <div className="identra-panel">
      <div className="identra-panel__head">
        <span className="identra-panel__tab" data-on="true">
          Connections
        </span>
        <button
          className="identra-panel__close"
          onClick={onClose}
          title="Close"
        >
          &times;
        </button>
      </div>
      <div className="identra-panel__list">
        {edges.length === 0 ? (
          <p className="identra-panel__empty">
            Nothing here is connected. Agents can only read each other's work
            once you connect them — or once one of them connects itself, which
            would show up here.
          </p>
        ) : (
          edges.map((e) => {
            const from = name(e.source);
            const to = name(e.target);
            return (
              <div className="identra-conn" key={e.id} data-by={e.by}>
                <div className="identra-conn__pair">
                  <AgentIcon kind={from.kind} className="identra-node__icon" />
                  <span className="identra-conn__name">{from.title}</span>
                  <span className="identra-conn__arrow">→</span>
                  <AgentIcon kind={to.kind} className="identra-node__icon" />
                  <span className="identra-conn__name">{to.title}</span>
                </div>
                <div className="identra-conn__meta">
                  <span>{SAID[e.by]}</span>
                  <button
                    className="identra-conn__revoke"
                    // Said in full, because the effect is not instant everywhere. A CLI reads its
                    // MCP servers once at startup, but `get_peer_context` re-reads this slice on
                    // every call, so a revoke does stop the reading — it just cannot un-read what
                    // has already been read into a running conversation.
                    title="Take this connection back. Neither agent can read the other's work from here on; anything already read stays in the conversation that read it."
                    onClick={() => onRevoke(e.id)}
                  >
                    Revoke
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
