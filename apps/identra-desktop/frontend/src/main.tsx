import { createRoot } from "react-dom/client";
import App from "./App";
import UpdateStrip from "./UpdateStrip";
import "./styles.css";

// No StrictMode: it double-invokes effects in dev, which would restart every terminal node.
// The update strip mounts beside the app, not inside it, so one launch checks once however the
// picker and the canvas trade places.
createRoot(document.getElementById("root")!).render(
  <>
    <UpdateStrip />
    <App />
  </>,
);
