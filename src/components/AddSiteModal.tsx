import { useEffect, useState } from "react";
import * as client from "../client";
import { browseFolder } from "../open";
import type {
  CreateFromTemplateResult,
  InstalledPackage,
  SiteTemplateInfo,
  SiteTemplateKind,
  TemplateRecord,
} from "../types";
import { AsyncModal, BusyLabel, useAsyncJob } from "./AsyncModal";

function runtimeValue(p: InstalledPackage): string {
  return p.version;
}

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

type SavedPick = { type: "saved"; id: string; label: string };
type KindPick = { type: "kind"; kind: SiteTemplateKind; label: string };
type StarterPick = KindPick | SavedPick;

const NEEDS_DB: SiteTemplateKind[] = [
  "wordpress",
  "woocommerce",
  "laravel",
  "symfony",
  "statamic",
  "drupal",
  "joomla",
];

const NEEDS_NODE: SiteTemplateKind[] = ["nextjs", "react", "vue", "angular"];

export function AddSiteModal({
  phpInstalled,
  nodeInstalled,
  savedTemplates,
  onClose,
  onCreatedFromTemplate,
}: {
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  savedTemplates: TemplateRecord[];
  onClose: () => void;
  /** Kept for call-site symmetry; starters always return a template result today. */
  onCreatedSite?: (siteId: string) => Promise<void>;
  onCreatedFromTemplate: (result: CreateFromTemplateResult) => Promise<void>;
}) {
  const job = useAsyncJob();
  const [starters, setStarters] = useState<SiteTemplateInfo[]>([]);
  const [pick, setPick] = useState<StarterPick>({
    type: "kind",
    kind: "empty",
    label: "Empty folder",
  });
  const [name, setName] = useState("my-site");
  const [path, setPath] = useState("~/Sites/my-site");
  const [php, setPhp] = useState(
    phpInstalled[0] ? runtimeValue(phpInstalled[0]) : "",
  );
  const [node, setNode] = useState(
    nodeInstalled[0] ? runtimeValue(nodeInstalled[0]) : "",
  );
  const [webServer, setWebServer] = useState("nginx");
  const [https, setHttps] = useState(true);
  const [engine, setEngine] = useState("mariadb");
  const [pickError, setPickError] = useState<string | null>(null);
  const [logLines, setLogLines] = useState<string[]>([]);

  useEffect(() => {
    void client
      .listSiteStarterKinds()
      .then((list) => {
        setStarters(list);
        if (list[0]) {
          setPick({ type: "kind", kind: list[0].kind, label: list[0].label });
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (pick.type !== "kind" || pick.kind === "park") return;
    const slug = slugify(name) || "site";
    setPath(`~/Sites/${slug}`);
  }, [name, pick]);

  useEffect(() => {
    if (!job.progress) return;
    const line = [job.progress.phase, job.progress.message]
      .filter(Boolean)
      .join(": ");
    if (!line.trim()) return;
    setLogLines((prev) => {
      if (prev[prev.length - 1] === line) return prev;
      return [...prev.slice(-80), line];
    });
  }, [job.progress]);

  useEffect(() => {
    if (pick.type !== "kind") return;
    if (pick.kind === "wordpress" || pick.kind === "woocommerce") {
      setWebServer("apache");
    } else if (
      pick.kind === "laravel" ||
      pick.kind === "html" ||
      pick.kind === "empty" ||
      NEEDS_NODE.includes(pick.kind)
    ) {
      setWebServer("nginx");
    }
  }, [pick]);

  const pickFolder = async () => {
    setPickError(null);
    try {
      const folder = await browseFolder({ title: "Park existing site folder" });
      if (folder) {
        setPath(folder);
        setPick({ type: "kind", kind: "park", label: "Park existing folder" });
        if (!name || name === "my-site") {
          const base = folder.replace(/[\\/]+$/, "").split(/[\\/]/).pop();
          if (base) setName(base);
        }
      }
    } catch (e) {
      setPickError(e instanceof Error ? e.message : String(e));
    }
  };

  const kind = pick.type === "kind" ? pick.kind : null;
  const needsDb = kind != null && NEEDS_DB.includes(kind);
  const needsNode = kind != null && NEEDS_NODE.includes(kind);
  const needsPhp =
    kind != null &&
    kind !== "empty" &&
    kind !== "html" &&
    kind !== "park" &&
    !needsNode;

  const cancel = () => {
    if (job.busy) {
      void client.cancelSiteTemplateJob();
      return;
    }
    onClose();
  };

  const submit = () =>
    void job.run(async () => {
      setLogLines([]);
      setPickError(null);
      if (pick.type === "saved") {
        const result = await client.cloneSite({
          sourceId: pick.id,
          name: name.trim(),
          path: path.trim() || `~/Sites/${slugify(name.trim()) || "site"}`,
          phpVersion: php || undefined,
        });
        await onCreatedFromTemplate({
          site: result.site,
          kind: "laravel",
          projectPath: result.site.path,
          docroot: result.site.path,
          url: result.url,
          database: null,
          steps: [
            {
              name: "clone",
              ok: true,
              skipped: false,
              message: `Cloned saved template ${pick.label}`,
            },
          ],
          softFailed: false,
          notes: [`Saved template: ${pick.label}`],
        });
        return;
      }

      const result = await client.createSiteFromTemplate({
        kind: pick.kind,
        name: name.trim(),
        path: path.trim() || null,
        phpVersion: php || null,
        databaseEngine: needsDb ? engine : null,
        webServer,
        https,
        nodeVersion: needsNode ? node || null : null,
      });
      await onCreatedFromTemplate(result);
    });

  return (
    <AsyncModal
      title="Add site"
      description="Pick a starter, then name the site. DevStack uses its own PHP, Composer, Node, WP-CLI and database — never system WAMP tools."
      busy={job.busy}
      progress={job.progress}
      error={job.error ?? pickError}
      wide
      testId="add-site-modal"
      onClose={() => {
        if (!job.busy) onClose();
      }}
      actions={
        <>
          <button
            type="button"
            className="btn"
            data-testid="add-site-cancel"
            onClick={cancel}
          >
            {job.busy ? "Cancel job" : "Cancel"}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={job.busy || !name.trim() || (kind === "park" && !path.trim())}
            data-testid="add-site-submit"
            onClick={submit}
          >
            <BusyLabel
              busy={job.busy}
              idle="Create site"
              busyText="Creating…"
              progress={job.progress}
            />
          </button>
        </>
      }
    >
      <div className="field" style={{ marginTop: 0 }}>
        <span className="modal-check-title">Starter</span>
        <div className="template-kinds" role="radiogroup" aria-label="Starter">
          {starters.map((s) => (
            <button
              key={s.kind}
              type="button"
              role="radio"
              aria-checked={pick.type === "kind" && pick.kind === s.kind}
              className={`chip chip-toggle ${
                pick.type === "kind" && pick.kind === s.kind ? "on" : ""
              }`}
              data-testid={`starter-${s.kind}`}
              disabled={job.busy}
              onClick={() => {
                if (s.kind === "park") {
                  void pickFolder();
                  return;
                }
                setPick({ type: "kind", kind: s.kind, label: s.label });
              }}
            >
              {s.label}
            </button>
          ))}
          {savedTemplates.map((t) => (
            <button
              key={`saved-${t.id}`}
              type="button"
              role="radio"
              aria-checked={pick.type === "saved" && pick.id === t.id}
              className={`chip chip-toggle ${
                pick.type === "saved" && pick.id === t.id ? "on" : ""
              }`}
              data-testid={`starter-saved-${t.id}`}
              disabled={job.busy}
              onClick={() =>
                setPick({
                  type: "saved",
                  id: t.id,
                  label: `Saved · ${t.name}`,
                })
              }
            >
              Saved · {t.name}
            </button>
          ))}
        </div>
        {pick.type === "kind" ? (
          <p className="hint" style={{ marginTop: 6 }}>
            {(starters.find((s) => s.kind === pick.kind)?.commands ?? []).join(
              " · ",
            ) || pick.label}
          </p>
        ) : (
          <p className="hint" style={{ marginTop: 6 }}>
            Clone your saved template into a new site folder.
          </p>
        )}
      </div>

      <label className="field">
        Name
        <input
          value={name}
          disabled={job.busy}
          data-testid="add-site-name"
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <label className="field">
        Directory
        <div style={{ display: "flex", gap: 8 }}>
          <input
            style={{ flex: 1 }}
            value={path}
            disabled={job.busy}
            data-testid="add-site-path"
            onChange={(e) => setPath(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-install"
            disabled={job.busy}
            title="Browse for a folder"
            onClick={() => void pickFolder()}
          >
            Browse
          </button>
        </div>
      </label>

      <div className="field-row">
        {needsPhp || needsDb ? (
          <label className="field">
            PHP
            {phpInstalled.length === 0 ? (
              <span className="hint">
                No PHP installed —{" "}
                <a href="#/php" onClick={onClose}>
                  install one
                </a>
              </span>
            ) : (
              <select
                className="native"
                value={php}
                disabled={job.busy}
                onChange={(e) => setPhp(e.target.value)}
              >
                {phpInstalled.map((v) => (
                  <option key={v.version} value={runtimeValue(v)}>
                    {runtimeValue(v)}
                  </option>
                ))}
              </select>
            )}
          </label>
        ) : null}
        {needsNode ? (
          <label className="field">
            Node
            {nodeInstalled.length === 0 ? (
              <span className="hint">
                No Node installed —{" "}
                <a href="#/javascript" onClick={onClose}>
                  install one
                </a>
              </span>
            ) : (
              <select
                className="native"
                value={node}
                disabled={job.busy}
                onChange={(e) => setNode(e.target.value)}
              >
                {nodeInstalled.map((v) => (
                  <option key={v.version} value={runtimeValue(v)}>
                    {runtimeValue(v)}
                  </option>
                ))}
              </select>
            )}
          </label>
        ) : null}
        <label className="field">
          Web server
          <select
            className="native"
            value={webServer}
            disabled={job.busy}
            onChange={(e) => setWebServer(e.target.value)}
          >
            <option value="nginx">Nginx</option>
            <option value="apache">Apache</option>
          </select>
        </label>
        {needsDb ? (
          <label className="field">
            Database
            <select
              className="native"
              value={engine}
              disabled={job.busy}
              onChange={(e) => setEngine(e.target.value)}
            >
              <option value="mariadb">MariaDB</option>
              <option value="mysql">MySQL</option>
            </select>
          </label>
        ) : null}
      </div>

      <label className="field" style={{ flexDirection: "row", gap: 8 }}>
        <input
          type="checkbox"
          checked={https}
          disabled={job.busy}
          onChange={(e) => setHttps(e.target.checked)}
        />
        HTTPS (.test certificate)
      </label>

      {job.busy || logLines.length ? (
        <pre className="code-block starter-log" data-testid="add-site-log">
          {logLines.join("\n") || "Starting…"}
        </pre>
      ) : null}
    </AsyncModal>
  );
}
