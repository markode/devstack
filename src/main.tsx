import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import * as client from "./client";

function installFrontendLogHooks() {
  const origError = console.error.bind(console);
  const origWarn = console.warn.bind(console);
  console.error = (...args: unknown[]) => {
    origError(...args);
    void client
      .appendAppLog(
        "error",
        "console",
        args.map((a) => (a instanceof Error ? a.stack ?? a.message : String(a))).join(" "),
      )
      .catch(() => {});
  };
  console.warn = (...args: unknown[]) => {
    origWarn(...args);
    void client
      .appendAppLog(
        "warn",
        "console",
        args.map((a) => String(a)).join(" "),
      )
      .catch(() => {});
  };
  window.addEventListener("error", (ev) => {
    void client
      .appendAppLog(
        "error",
        "console",
        ev.error instanceof Error
          ? ev.error.stack ?? ev.error.message
          : String(ev.message),
      )
      .catch(() => {});
  });
  window.addEventListener("unhandledrejection", (ev) => {
    const reason = ev.reason;
    void client
      .appendAppLog(
        "error",
        "console",
        reason instanceof Error
          ? reason.stack ?? reason.message
          : `unhandledrejection ${String(reason)}`,
      )
      .catch(() => {});
  });
}

installFrontendLogHooks();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
