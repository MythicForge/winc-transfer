import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/tokens.css";
import "./styles/global.css";
import "./styles/app.css";
import "./styles/log.css";
import App from "./App.tsx";
import LogWindow from "./LogWindow.tsx";
import { StoreProvider } from "./store";
import { initLogBus, isLogWindow } from "./lib/log";

initLogBus();

// One bundle, two roots: the popout loads the same index.html with ?view=log.
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isLogWindow ? (
      <LogWindow />
    ) : (
      <StoreProvider>
        <App />
      </StoreProvider>
    )}
  </StrictMode>,
);
