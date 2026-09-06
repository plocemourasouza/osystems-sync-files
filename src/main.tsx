import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { App } from "./App";
import "./styles/app.css";

const rootElement = document.getElementById("root");

if (!rootElement) {
  throw new Error("Root element not found");
}

async function bootstrap(root: HTMLElement): Promise<void> {
  // Dev-only mock mode (`?mock=1`): seeds every store with fixture data
  // before the first render, so the Dashboard/Settings screens can be
  // previewed without a running Tauri backend (real-device 1366×768 bug
  // hunting, `JobTable.tsx`'s colgroup comment). `import.meta.env.DEV` is a
  // compile-time constant Vite folds to `false` in production, so this
  // whole branch — the dynamic imports included — is dead-code-eliminated
  // from the production bundle.
  if (import.meta.env.DEV) {
    const { isMockMode } = await import("./dev/mockMode");
    if (isMockMode()) {
      const { seedMockData } = await import("./dev/mock");
      seedMockData();
    }
  }

  createRoot(root).render(
    <StrictMode>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </StrictMode>,
  );
}

void bootstrap(rootElement);
