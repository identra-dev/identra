// What a workspace tile draws now that there is no board to draw.
//
// This used to be the arrangement in miniature: nodes as rectangles where the user had put them,
// wires and all. That thumbnail was worth having because the arrangement was the thing a person
// recognised their project by. With the canvas gone there is no arrangement, and drawing the same
// rectangles out of a file where every position is the origin would put one identical stack of
// boxes on every tile in the picker — the same picture for every project, which is worse than no
// picture at all.
//
// So the tile says what is actually in there: which agents this workspace has open, in their own
// colours, and how many. That differs between projects and it is something a person can recognise,
// which was the whole job the thumbnail was doing.
import type { Canvas } from "./api";
import { AgentIcon } from "./icons";

type Props = {
  canvas: Canvas;
  className?: string;
};

// How many icons fit before the tile stops being a glance. Past this it becomes a count, which
// reads faster than nine overlapping squares anyway.
const SHOWN = 4;

export default function BoardPreview({
  canvas,
  className = "identra-preview",
}: Props) {
  // Agents and dev servers only. A file viewer, a note and a web view are things the workspace
  // holds, not things running in it, and the question this tile answers is what is going on here.
  const running = canvas.nodes.filter(
    (n) => n.kind !== "file" && n.kind !== "note" && n.kind !== "browser",
  );
  return (
    <span className={className} aria-hidden="true">
      {running.slice(0, SHOWN).map((n) => (
        <AgentIcon key={n.id} kind={n.kind} className="identra-preview__tile" />
      ))}
      {running.length > SHOWN && (
        <span className="identra-preview__more">+{running.length - SHOWN}</span>
      )}
    </span>
  );
}
