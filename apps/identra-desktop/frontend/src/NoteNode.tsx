import { memo } from "react";
import { type NodeProps } from "@xyflow/react";
import type { AgentNodeData } from "./AgentNode";

// A note on the canvas. No PTY, no ports: it is a thing to read, not a thing to wire. An agent
// leaves one when it has something a human should decide on, and the text rides in the node's
// title so it saves and reloads with the canvas like any other node, with no schema change.
function NoteNodeImpl({ data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  return (
    <div className="identra-note">
      <div className="identra-note__body">{nodeData.title}</div>
    </div>
  );
}

export default memo(NoteNodeImpl);
