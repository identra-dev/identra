import { createRoot } from "react-dom/client";
import App from "./App";
import Greeting from "./Greeting";
import UpdateStrip from "./UpdateStrip";
import "./styles.css";

// No StrictMode: it double-invokes effects in dev, which would restart every terminal node.
// The update strip mounts beside the app, not inside it, so one launch checks once however the
// picker and the canvas trade places.
//
// The greeting is here for exactly the same reason, and it is the reason rather than the tidiness
// that matters. Inside App it would sit in two different branches — one beside the picker, one over
// the canvas — and React unmounts and remounts across that swap, so opening a workspace would
// re-read the settings and say good morning a second time. Out here it mounts once, when the app
// does, which is also the only moment it is describing.
createRoot(document.getElementById("root")!).render(
  <>
    <Greeting />
    <UpdateStrip />
    <App />
  </>,
);
