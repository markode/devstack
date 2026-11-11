import { useEffect, useMemo, useState } from "react";
import * as client from "../client";
import { browseFolder } from "../open";
import {
  applyStarterDefaults,
  getStarterRecommendation,
  incompatibleWarning,
  pickInstallVersion,
  resolveRuntimeChoice,
  sameMinorFamily,
  type RuntimeReq,
} from "../starterRecommendations";
import type {
  CreateFromTemplateResult,
  InstalledPackage,
  PackageVersion,
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

function RecommendedBadge({ show }: { show: boolean }) {
  if (!show) return null;
  return (
    <span className="rec-badge" data-testid="recommended-badge">
      Recommended
    </span>
  );
}

function RuntimeField({
  label,
  packageId,
  value,
  onChange,
  installed,
  req,
  disabled,
  busy,
  onInstalled,
}: {
  label: string;
  packageId: "php" | "node";
  value: string;
  onChange: (v: string) => void;
  installed: InstalledPackage[];
  req: RuntimeReq | null;
  disabled?: boolean;
  busy?: boolean;
  onInstalled: (pkg: InstalledPackage) => void;
}) {
  const resolved = resolveRuntimeChoice(req, installed);
  const isRecommended = !!req && !!value && sameMinorFamily(value, req.recommended);
  const [installBusy, setInstallBusy] = useState(false);
  const [installMsg, setInstallMsg] = useState<string | null>(null);

  const installRecommended = async () => {
    if (!req) return;
    setInstallBusy(true);
    setInstallMsg(`Looking up ${label} ${req.recommended}…`);
    try {
      const list = await client.listPackageVersions(packageId);
      const version = pickInstallVersion(req, list.versions as PackageVersion[]);
      if (!version) {
        setInstallMsg(`No ${label} ${req.recommended} in the catalog.`);
        return;
      }
      setInstallMsg(`Installing ${label} ${version}…`);
      const rec = await client.installPackage(packageId, version);
      onInstalled(rec);
      onChange(rec.version);
      setInstallMsg(`Installed ${label} ${rec.version}.`);
    } catch (e) {
      setInstallMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setInstallBusy(false);
    }
  };

  const showInstall =
    !!req &&
    (resolved?.status === "missing" ||
      resolved?.status === "compatible" ||
      resolved?.status === "below-minimum");

  return (
    <label className="field" data-testid={`runtime-field-${packageId}`}>
      <span className="field-label-row">
        {label}
        <RecommendedBadge show={isRecommended} />
      </span>
      <select
        className="native"
        value={value}
        disabled={disabled || busy || installBusy}
        data-testid={`select-${packageId}`}
        onChange={(e) => onChange(e.target.value)}
      >
        {installed.length === 0 ? <option value="">— not installed —</option> : null}
        {installed.map((v) => (
          <option key={v.version} value={runtimeValue(v)}>
            {runtimeValue(v)}
            {req && sameMinorFamily(runtimeValue(v), req.recommended)
              ? " (recommended)"
              : ""}
          </option>
        ))}
      </select>
      {resolved?.note ? (
        <span className="hint" data-testid={`${packageId}-fallback-note`}>
          {resolved.note}
        </span>
      ) : null}
      {showInstall ? (
        <button
          type="button"
          className="btn btn-install"
          style={{ marginTop: 6 }}
          disabled={disabled || busy || installBusy}
          data-testid={`install-${packageId}-recommended`}
          onClick={() => void installRecommended()}
        >
          <BusyLabel
            busy={installBusy}
            idle={`Install ${label} ${req!.recommended}`}
            busyText={installMsg ?? "Installing…"}
          />
        </button>
      ) : null}
      {installMsg && !installBusy ? (
        <span className="hint" data-testid={`${packageId}-install-msg`}>
          {installMsg}
        </span>
      ) : null}
    </label>
  );
}

export function AddSiteModal({
  phpInstalled: phpInstalledProp,
  nodeInstalled: nodeInstalledProp,
  savedTemplates,
  onClose,
  onCreatedFromTemplate,
}: {
  phpInstalled: InstalledPackage[];
  nodeInstalled: InstalledPackage[];
  savedTemplates: TemplateRecord[];
  onClose: () => void;
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
  const [phpInstalled, setPhpInstalled] = useState(phpInstalledProp);
  const [nodeInstalled, setNodeInstalled] = useState(nodeInstalledProp);
  const [php, setPhp] = useState("");
  const [node, setNode] = useState("");
  const [webServer, setWebServer] = useState("nginx");
  const [https, setHttps] = useState(true);
  const [engine, setEngine] = useState("mariadb");
  const [pickError, setPickError] = useState<string | null>(null);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [dbInstalled, setDbInstalled] = useState<Record<string, boolean>>({
    mariadb: false,
    mysql: false,
  });
  const [dbInstallBusy, setDbInstallBusy] = useState(false);
  const [dbInstallMsg, setDbInstallMsg] = useState<string | null>(null);

  useEffect(() => {
    setPhpInstalled(phpInstalledProp);
  }, [phpInstalledProp]);
  useEffect(() => {
    setNodeInstalled(nodeInstalledProp);
  }, [nodeInstalledProp]);

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
    void Promise.all([
      client.listInstalled("mariadb").catch(() => []),
      client.listInstalled("mysql").catch(() => []),
    ]).then(([maria, mysql]) => {
      setDbInstalled({
        mariadb: maria.length > 0,
        mysql: mysql.length > 0,
      });
    });
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

  // Prefill recommended stack whenever the starter changes.
  useEffect(() => {
    if (pick.type !== "kind") return;
    const { webServer: ws, database, recommendation } = applyStarterDefaults(pick.kind);
    setWebServer(ws);
    if (database) setEngine(database);
    const phpRes = resolveRuntimeChoice(recommendation?.php ?? null, phpInstalled);
    setPhp(phpRes?.value ?? "");
    const nodeRes = resolveRuntimeChoice(recommendation?.node ?? null, nodeInstalled);
    setNode(nodeRes?.value ?? "");
  }, [pick, phpInstalled, nodeInstalled]);

  const kind = pick.type === "kind" ? pick.kind : null;
  const recommendation = kind ? getStarterRecommendation(kind) : null;
  const needsDb = !!recommendation?.database;
  const needsNode = !!recommendation?.node;
  const needsPhp = !!recommendation?.php;

  const warning = useMemo(() => {
    if (!kind) return null;
    return incompatibleWarning(kind, {
      php: needsPhp ? php : undefined,
      node: needsNode ? node : undefined,
      webServer,
      database: needsDb ? engine : undefined,
    });
  }, [kind, php, node, webServer, engine, needsPhp, needsNode, needsDb]);

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

  const installDb = async () => {
    const id = engine === "mysql" ? "mysql" : "mariadb";
    setDbInstallBusy(true);
    setDbInstallMsg(`Looking up ${id}…`);
    try {
      const list = await client.listPackageVersions(id);
      const version =
        list.versions.find((v) => v.lts)?.version ?? list.versions[0]?.version;
      if (!version) {
        setDbInstallMsg(`No ${id} versions in the catalog.`);
        return;
      }
      setDbInstallMsg(`Installing ${id} ${version}…`);
      await client.installPackage(id, version);
      setDbInstalled((prev) => ({ ...prev, [id]: true }));
      setDbInstallMsg(`Installed ${id} ${version}.`);
    } catch (e) {
      setDbInstallMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setDbInstallBusy(false);
    }
  };

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
        phpVersion: needsPhp ? php || null : null,
        databaseEngine: needsDb ? engine : null,
        webServer,
        https,
        nodeVersion: needsNode ? node || null : null,
      });
      await onCreatedFromTemplate(result);
    });

  const webServerIsRec = recommendation?.webServer === webServer;
  const dbIsRec = recommendation?.database?.recommended === engine;

  return (
    <AsyncModal
      title="Add site"
      description="Pick a starter — DevStack prefills the recommended stack. Override anytime. Uses managed PHP/Composer/Node/WP-CLI/DB only."
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
            disabled={
              job.busy || !name.trim() || (kind === "park" && !path.trim())
            }
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
        {recommendation?.notes ? (
          <p className="hint" style={{ marginTop: 6 }} data-testid="starter-rec-notes">
            {recommendation.notes}
            {recommendation.frameworkVersion
              ? ` · Framework ${recommendation.frameworkVersion}`
              : ""}
          </p>
        ) : pick.type === "saved" ? (
          <p className="hint" style={{ marginTop: 6 }}>
            Clone your saved template into a new site folder.
          </p>
        ) : null}
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
        {needsPhp ? (
          <RuntimeField
            label="PHP"
            packageId="php"
            value={php}
            onChange={setPhp}
            installed={phpInstalled}
            req={recommendation?.php ?? null}
            disabled={job.busy}
            busy={job.busy}
            onInstalled={(pkg) =>
              setPhpInstalled((prev) => {
                if (prev.some((p) => p.version === pkg.version)) return prev;
                return [...prev, pkg];
              })
            }
          />
        ) : null}
        {needsNode ? (
          <RuntimeField
            label="Node"
            packageId="node"
            value={node}
            onChange={setNode}
            installed={nodeInstalled}
            req={recommendation?.node ?? null}
            disabled={job.busy}
            busy={job.busy}
            onInstalled={(pkg) =>
              setNodeInstalled((prev) => {
                if (prev.some((p) => p.version === pkg.version)) return prev;
                return [...prev, pkg];
              })
            }
          />
        ) : null}
        <label className="field" data-testid="runtime-field-webserver">
          <span className="field-label-row">
            Web server
            <RecommendedBadge show={webServerIsRec} />
          </span>
          <select
            className="native"
            value={webServer}
            disabled={job.busy}
            data-testid="select-webserver"
            onChange={(e) => setWebServer(e.target.value)}
          >
            <option value="nginx">
              Nginx{recommendation?.webServer === "nginx" ? " (recommended)" : ""}
            </option>
            <option value="apache">
              Apache
              {recommendation?.webServer === "apache" ? " (recommended)" : ""}
            </option>
          </select>
        </label>
        {needsDb ? (
          <label className="field" data-testid="runtime-field-database">
            <span className="field-label-row">
              Database
              <RecommendedBadge show={dbIsRec} />
            </span>
            <select
              className="native"
              value={engine}
              disabled={job.busy || dbInstallBusy}
              data-testid="select-database"
              onChange={(e) => setEngine(e.target.value)}
            >
              <option value="mariadb">
                MariaDB
                {recommendation?.database?.recommended === "mariadb"
                  ? " (recommended)"
                  : ""}
              </option>
              <option value="mysql">
                MySQL
                {recommendation?.database?.recommended === "mysql"
                  ? " (recommended)"
                  : ""}
              </option>
            </select>
            {!dbInstalled[engine] ? (
              <>
                <span className="hint" data-testid="db-fallback-note">
                  {engine} is not installed yet.
                </span>
                <button
                  type="button"
                  className="btn btn-install"
                  style={{ marginTop: 6 }}
                  disabled={job.busy || dbInstallBusy}
                  data-testid="install-database-recommended"
                  onClick={() => void installDb()}
                >
                  <BusyLabel
                    busy={dbInstallBusy}
                    idle={`Install ${engine}`}
                    busyText={dbInstallMsg ?? "Installing…"}
                  />
                </button>
              </>
            ) : null}
            {dbInstallMsg && !dbInstallBusy ? (
              <span className="hint">{dbInstallMsg}</span>
            ) : null}
          </label>
        ) : null}
      </div>

      {warning ? (
        <div
          className="port-notice-banner"
          role="status"
          data-testid="starter-compat-warning"
          style={{ marginTop: 12 }}
        >
          {warning}
        </div>
      ) : null}

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
