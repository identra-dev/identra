// A file on the canvas, read-only: the artifact an agent hands over, or a file the user dropped
// on to look at. The path rides in `cwd` the same way the browser node carries its URL, so it
// saves and reloads with the canvas and needs no schema change.
import { memo, useEffect, useState } from "react";
import { Handle, Position, useReactFlow, type NodeProps } from "@xyflow/react";
import type { AgentNodeData } from "./AgentNode";
import { fileRead, type FileView } from "./api";
import { AgentIcon } from "./icons";

type State =
  | { at: "loading" }
  | { at: "error"; why: string }
  | { at: "ready"; view: FileView; imageUrl: string | null };

function FileNodeImpl({ id, data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  const { deleteElements } = useReactFlow();
  const [state, setState] = useState<State>({ at: "loading" });

  useEffect(() => {
    let dropped = false;
    let url: string | null = null;
    const path = nodeData.cwd ?? "";
    fileRead(path).then(
      (view) => {
        if (dropped) return;
        if (view.kind === "image") {
          // A blob URL, revoked when the node goes, so an image is bytes exactly once and never
          // a base64 string glued into the DOM.
          url = URL.createObjectURL(new Blob([new Uint8Array(view.bytes)]));
        }
        setState({ at: "ready", view, imageUrl: url });
      },
      (e) => {
        if (!dropped) setState({ at: "error", why: String(e) });
      },
    );
    return () => {
      dropped = true;
      if (url !== null) URL.revokeObjectURL(url);
    };
  }, [nodeData.cwd]);

  const name =
    state.at === "ready" ? state.view.name : nodeData.title || "file";

  return (
    <div className="identra-node">
      <Handle type="target" position={Position.Left} className="identra-port" />
      <Handle
        type="source"
        position={Position.Right}
        className="identra-port"
      />
      <div className="identra-node__header">
        <AgentIcon kind="file" className="identra-node__icon" />
        <span className="identra-node__title">{name}</span>
        <button
          className="identra-node__close nodrag"
          title={`Close ${name}`}
          onClick={() => void deleteElements({ nodes: [{ id }] })}
        >
          &times;
        </button>
      </div>
      <div className="identra-file nodrag nowheel">
        {state.at === "loading" && (
          <p className="identra-file__state">Reading...</p>
        )}
        {state.at === "error" && (
          <p className="identra-file__state" role="alert">
            {state.why}
          </p>
        )}
        {state.at === "ready" && state.view.kind === "text" && (
          <pre className="identra-file__text">{state.view.text}</pre>
        )}
        {state.at === "ready" && state.view.kind === "image" && (
          <img
            className="identra-file__image"
            src={state.imageUrl ?? ""}
            alt={state.view.name}
          />
        )}
        {state.at === "ready" && state.view.kind === "binary" && (
          <p className="identra-file__state">
            {state.view.name} is a binary file ({state.view.size} bytes), so
            there is nothing to read here.
          </p>
        )}
        {state.at === "ready" && state.view.kind === "toobig" && (
          <p className="identra-file__state">
            {state.view.name} is {Math.round(state.view.size / 1024)}KB, which
            is more than this viewer will load. Open it in your editor.
          </p>
        )}
      </div>
    </div>
  );
}

export default memo(FileNodeImpl);
