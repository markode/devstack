import { useEffect, useState } from "react";
import * as client from "../client";
import type { AppInfo } from "../types";

export function GeneralPage() {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void client.getAppInfo().then(setInfo).catch(() => setInfo(null));
  }, []);

  return (
    <div className="stack">
      <div className="hero-card">
        <div
          className="hero-icon"
          style={{
            background: "linear-gradient(145deg, #2f6bff, #0a84ff 55%, #64d2ff)",
          }}
        >
          DS
        </div>
        <h1>DevStack</h1>
        <p>
          Local PHP development environment for macOS and Windows — runtimes,
          databases, and sites in one place.
        </p>
      </div>

      <div className="panel-card settings-list">
        <div className="settings-row">
          <label>
            Version
            <span className="hint">Application build</span>
          </label>
          <span className="muted">{info?.version ?? "…"}</span>
        </div>
        <div className="settings-row">
          <label>
            Platform
            <span className="hint">Detected host</span>
          </label>
          <span className="muted">
            {info ? `${info.os}/${info.arch}` : "…"}
          </span>
        </div>
        <div className="settings-row">
          <label>
            Data directory
            <span className="hint">Packages &amp; state</span>
          </label>
          <span className="muted" style={{ fontSize: 11, maxWidth: 280, textAlign: "right" }}>
            {info?.dataDir ?? "…"}
          </span>
        </div>
        <div className="settings-row">
          <label>
            Preview mode
            <span className="hint">
              {client.useMock()
                ? "UI mock (open via Tauri for live installs)"
                : "Connected to Rust core"}
            </span>
          </label>
          <span className="muted">{client.useMock() ? "Mock" : "Live"}</span>
        </div>
      </div>
    </div>
  );
}

export function PlaceholderPage({
  title,
  blurb,
}: {
  title: string;
  blurb: string;
}) {
  return (
    <div className="hero-card">
      <h1>{title}</h1>
      <p className="placeholder-note">{blurb}</p>
    </div>
  );
}
