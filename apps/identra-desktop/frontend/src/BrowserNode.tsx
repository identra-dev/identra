import { memo, useState } from "react";
import { Handle, Position, useReactFlow, type NodeProps } from "@xyflow/react";
import type { AgentNodeData } from "./AgentNode";
import { iconFor } from "./icons";

// A web view on the canvas: same window chrome as an agent node, but the body is an iframe and
// there is no PTY — kind === "browser" never calls terminal_start. The URL rides in the node's
// `cwd` field so it saves and reloads exactly like a node's working dir, no schema change.
function BrowserNodeImpl({ id, data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  const { updateNodeData } = useReactFlow();
  // `url` is what the iframe loads; `draft` is what's being typed. Splitting them keeps the frame
  // from reloading on every keystroke — it navigates only when a URL is committed.
  const [url, setUrl] = useState(nodeData.cwd || "");
  const [draft, setDraft] = useState(url);

  const commit = (next: string) => {
    setUrl(next);
    updateNodeData(id, { cwd: next }); // routes through onNodesChange, so it saves with the canvas
  };

  const icon = iconFor("browser");
  return (
    <div className="identra-node">
      <Handle type="target" position={Position.Left} className="identra-port" />
      <Handle
        type="source"
        position={Position.Right}
        className="identra-port"
      />
      <div className="identra-node__header">
        <span className="identra-node__icon" style={{ background: icon.tile }}>
          {icon.glyph}
        </span>
        <input
          className="identra-node__url nodrag"
          value={draft}
          spellCheck={false}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget.value);
          }}
          onBlur={(e) => commit(e.currentTarget.value)}
        />
      </div>
      <iframe
        className="identra-node__frame nodrag nowheel"
        src={url}
        title={nodeData.title || "browser"}
      />
    </div>
  );
}

export default memo(BrowserNodeImpl);
