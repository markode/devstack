import { useCallback, useEffect, useId, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import { Modal } from "../components/Modal";
import * as client from "../client";
import { humanizeError } from "../portBusy";
import type {
  HostingProfile,
  HostingTools,
  SiteRecord,
  SyncDirection,
  SyncPlan,
  SyncResult,
} from "../types";
import { formatBytes, splitExcludes, syncPlanSummary } from "../v030";

const EMPTY: HostingProfile = {
  id: "",
  name: "",
  host: "",
  port: 22,
  user: "",
  auth: "key",
  keyPath: "",
  remotePath: "/var/www/html",
  remoteUrl: "",
  siteId: "",
  excludes: [],
  remoteDb: null,
};

/** SFTP/SSH deploy: profiles, dry-run preview, pull/push (+ optional database). */
export function HostingPage() {
  const [tools, setTools] = useState<HostingTools | null>(null);
  const [profiles, setProfiles] = useState<HostingProfile[]>([]);
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [editing, setEditing] = useState<HostingProfile | null>(null);
  const [password, setPassword] = useState("");
  const [dbPassword, setDbPassword] = useState("");
  const [excludeText, setExcludeText] = useState("");
  const [plan, setPlan] = useState<SyncPlan | null>(null);
  const [result, setResult] = useState<SyncResult | null>(null);
  const [withDb, setWithDb] = useState(false);
  const [del, setDel] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [t, p, s] = await Promise.allSettled([
      client.hostingToolsStatus(),
      client.hostingListProfiles(),
      client.getSites(),
    ]);
    if (t.status === "fulfilled") setTools(t.value);
    if (p.status === "fulfilled") setProfiles(p.value);
    if (s.status === "fulfilled") setSites(s.value);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void client
      .onJobProgress((ev) => {
        if (ev.jobId === "hosting-sync") setProgress(`${ev.phase}: ${ev.message}`);
      })
      .then((u) => {
        unlisten = u;
      });
    return () => unlisten?.();
  }, []);

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await fn();
    } catch (e) {
      setError(humanizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  const startEdit = (p: HostingProfile) => {
    setEditing({ ...p });
    setExcludeText((p.excludes ?? []).join("\n"));
    setPassword("");
    setDbPassword("");
  };

  const save = () =>
    guard(async () => {
      if (!editing) return;
      const saved = await client.hostingSaveProfile(
        {
          ...editing,
          excludes: splitExcludes(excludeText),
          siteId: editing.siteId || null,
          remoteDb: editing.remoteDb?.name ? editing.remoteDb : null,
        },
        password || null,
        dbPassword || null,
      );
      setEditing(null);
      setMessage(`Saved ${saved.name}`);
      await load();
    });

  const runPreview = (p: HostingProfile, dir: SyncDirection) =>
    guard(async () => {
      setResult(null);
      setPlan(await client.hostingPreview(p.id, dir, del));
    });

  const runSync = (p: HostingProfile, dir: SyncDirection) =>
    guard(async () => {
      const warn =
        dir === "push"
          ? "Push uploads local files to the server" + (del ? " and DELETES remote files that do not exist locally" : "") + (withDb ? " and REPLACES the remote database (a backup is taken first)" : "") + ". Continue?"
          : "Pull overwrites local files" + (withDb ? " and replaces the local database" : "") + ". Continue?";
      if (!window.confirm(warn)) return;
      setResult(await client.hostingRunSync(p.id, dir, { delete: del, includeDatabase: withDb }));
      setPlan(null);
    });

  return (
    <div data-testid="hosting-page">
      <SectionHeader
        title="Hosting"
        description="Pull or push a site over SSH/SFTP with excludes, a dry-run preview and WordPress URL search-replace."
      />
      {tools && tools.notes.length > 0 ? (
        <div className="port-notice-banner" role="status">
          {tools.notes.map((n) => (
            <p key={n}>{n}</p>
          ))}
        </div>
      ) : null}
      {error ? <div className="error-banner">{error}</div> : null}
      {message ? <div className="port-notice-banner">{message}</div> : null}
      {progress ? <div className="loading">{progress}</div> : null}

      <div className="panel-card settings-list">
        <div className="settings-row settings-row-head">
          <label>Connections</label>
          <button type="button" className="btn btn-primary" onClick={() => startEdit({ ...EMPTY })}>
            Add connection
          </button>
        </div>
        {profiles.length === 0 ? (
          <div className="settings-row">
            <span className="muted">No connections yet.</span>
          </div>
        ) : null}
        {profiles.map((p) => (
          <div key={p.id} className="settings-row settings-row-block" data-testid={`hosting-profile-${p.id}`}>
            <div className="row-actions" style={{ justifyContent: "space-between", width: "100%" }}>
              <label>
                {p.name}
                <span className="hint">
                  {p.user}@{p.host}:{p.port} → {p.remotePath}
                  {p.siteId ? ` · site ${p.siteId}` : " · not linked to a site"}
                </span>
              </label>
              <div className="row-actions">
                <button type="button" className="btn btn-install" disabled={busy} onClick={() => void guard(async () => setMessage(await client.hostingTestConnection(p.id)))}>
                  Test
                </button>
                <button type="button" className="btn btn-install" disabled={busy || !p.siteId} onClick={() => void runPreview(p, "pull")}>
                  Preview pull
                </button>
                <button type="button" className="btn btn-install" disabled={busy || !p.siteId} onClick={() => void runPreview(p, "push")}>
                  Preview push
                </button>
                <button type="button" className="btn btn-install" onClick={() => startEdit(p)}>
                  Edit
                </button>
                <button
                  type="button"
                  className="btn btn-install"
                  disabled={busy}
                  onClick={() => {
                    if (window.confirm(`Delete connection ${p.name}?`)) void guard(async () => {
                      await client.hostingDeleteProfile(p.id);
                      await load();
                    });
                  }}
                >
                  Delete
                </button>
              </div>
            </div>
            {plan && plan.profileId === p.id ? (
              <div style={{ width: "100%" }} data-testid="hosting-plan">
                <p>
                  <strong>{plan.direction === "pull" ? "Pull" : "Push"} preview</strong> · {syncPlanSummary(plan)} · via {plan.tool}
                </p>
                {plan.notes.map((n) => (
                  <p key={n} className="hint">
                    {n}
                  </p>
                ))}
                <div className="site-log-list">
                  {plan.entries.filter((e) => !e.isDir).slice(0, 200).map((e) => (
                    <div key={`${e.action}-${e.path}`} className={`site-log-entry lvl-${e.action === "delete" ? "error" : e.action === "add" ? "info" : "notice"}`}>
                      <span className="site-log-level">{e.action}</span>
                      <span className="site-log-msg">
                        {e.path} {e.action !== "delete" ? `(${formatBytes(e.size)})` : ""}
                      </span>
                    </div>
                  ))}
                </div>
                <div className="row-actions" style={{ marginTop: 8 }}>
                  <label className="chip chip-toggle">
                    <input type="checkbox" checked={del} onChange={(e) => setDel(e.target.checked)} /> Delete extraneous
                  </label>
                  <label className="chip chip-toggle">
                    <input type="checkbox" checked={withDb} disabled={!p.remoteDb} onChange={(e) => setWithDb(e.target.checked)} /> Include database
                  </label>
                  <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void runSync(p, plan.direction)}>
                    Run {plan.direction}
                  </button>
                </div>
              </div>
            ) : null}
          </div>
        ))}
      </div>

      {result ? (
        <div className="panel-card settings-list" style={{ marginTop: 16 }} data-testid="hosting-result">
          <div className="settings-row settings-row-block">
            <strong>{result.direction === "pull" ? "Pulled" : "Pushed"} {result.filesTransferred} file(s)</strong>
            <span className="hint">
              {formatBytes(result.bytes)} · {result.deleted} deleted
              {result.databaseSynced ? " · database synced" : ""}
            </span>
            {result.urlRewritten ? <span className="hint">{result.urlRewritten}</span> : null}
            {result.notes.map((n) => (
              <span key={n} className="hint">
                {n}
              </span>
            ))}
          </div>
        </div>
      ) : null}

      {editing ? (
        <HostingEditModal
          editing={editing}
          setEditing={setEditing}
          password={password}
          setPassword={setPassword}
          excludeText={excludeText}
          setExcludeText={setExcludeText}
          dbPassword={dbPassword}
          setDbPassword={setDbPassword}
          sites={sites}
          busy={busy}
          onSave={() => void save()}
          onCancel={() => setEditing(null)}
        />
      ) : null}
    </div>
  );
}

function HostingEditModal({
  editing,
  setEditing,
  password,
  setPassword,
  excludeText,
  setExcludeText,
  dbPassword,
  setDbPassword,
  sites,
  busy,
  onSave,
  onCancel,
}: {
  editing: HostingProfile;
  setEditing: (p: HostingProfile) => void;
  password: string;
  setPassword: (v: string) => void;
  excludeText: string;
  setExcludeText: (v: string) => void;
  dbPassword: string;
  setDbPassword: (v: string) => void;
  sites: SiteRecord[];
  busy: boolean;
  onSave: () => void;
  onCancel: () => void;
}) {
  const titleId = useId();
  return (
    <Modal
      onClose={onCancel}
      busy={busy}
      labelledBy={titleId}
      wide
      initialFocus="[data-autofocus]"
    >
      <h3 id={titleId}>{editing.id ? "Edit connection" : "New connection"}</h3>
      <label className="field">
        Name
        <input
          data-autofocus
          value={editing.name}
          onChange={(e) => setEditing({ ...editing, name: e.target.value })}
        />
      </label>
      <div className="field-row">
        <label className="field">
          Host
          <input value={editing.host} onChange={(e) => setEditing({ ...editing, host: e.target.value })} />
        </label>
        <label className="field" style={{ maxWidth: 90 }}>
          Port
          <input
            type="number"
            value={editing.port}
            onChange={(e) => setEditing({ ...editing, port: Number(e.target.value) })}
          />
        </label>
        <label className="field">
          User
          <input value={editing.user} onChange={(e) => setEditing({ ...editing, user: e.target.value })} />
        </label>
      </div>
      <div className="field-row">
        <label className="field">
          Auth
          <select
            className="native"
            value={editing.auth}
            onChange={(e) =>
              setEditing({ ...editing, auth: e.target.value as "key" | "password" })
            }
          >
            <option value="key">SSH key / agent</option>
            <option value="password">Password (OS keychain)</option>
          </select>
        </label>
        {editing.auth === "key" ? (
          <label className="field">
            Key file (optional)
            <input
              value={editing.keyPath ?? ""}
              onChange={(e) => setEditing({ ...editing, keyPath: e.target.value })}
            />
          </label>
        ) : (
          <label className="field">
            Password {editing.hasPassword ? "(stored — leave empty to keep)" : ""}
            <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
          </label>
        )}
      </div>
      <label className="field">
        Remote folder
        <input
          value={editing.remotePath}
          onChange={(e) => setEditing({ ...editing, remotePath: e.target.value })}
        />
      </label>
      <div className="field-row">
        <label className="field">
          Remote site URL
          <input
            placeholder="https://example.com"
            value={editing.remoteUrl ?? ""}
            onChange={(e) => setEditing({ ...editing, remoteUrl: e.target.value })}
          />
        </label>
        <label className="field">
          Local site
          <select
            className="native"
            value={editing.siteId ?? ""}
            onChange={(e) => setEditing({ ...editing, siteId: e.target.value })}
          >
            <option value="">— none —</option>
            {sites.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      <label className="field">
        Excludes (one per line)
        <textarea
          rows={4}
          value={excludeText}
          placeholder={".git/\nnode_modules/\n*.log"}
          onChange={(e) => setExcludeText(e.target.value)}
          style={{ fontFamily: "var(--mono)", fontSize: 12 }}
        />
      </label>
      <div className="field-row">
        <label className="field">
          Remote DB name
          <input
            value={editing.remoteDb?.name ?? ""}
            onChange={(e) =>
              setEditing({
                ...editing,
                remoteDb: {
                  host: editing.remoteDb?.host ?? "",
                  user: editing.remoteDb?.user ?? "",
                  name: e.target.value,
                },
              })
            }
          />
        </label>
        <label className="field">
          DB user
          <input
            value={editing.remoteDb?.user ?? ""}
            onChange={(e) =>
              setEditing({
                ...editing,
                remoteDb: {
                  host: editing.remoteDb?.host ?? "",
                  name: editing.remoteDb?.name ?? "",
                  user: e.target.value,
                },
              })
            }
          />
        </label>
        <label className="field">
          DB password
          <input type="password" value={dbPassword} onChange={(e) => setDbPassword(e.target.value)} />
        </label>
      </div>
      <div className="modal-actions">
        <button type="button" className="btn btn-install" disabled={busy} onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || !editing.name || !editing.host || !editing.user}
          onClick={onSave}
        >
          Save
        </button>
      </div>
    </Modal>
  );
}
