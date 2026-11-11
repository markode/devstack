import { useEffect, useId, useState } from "react";
import * as client from "../client";
import { AsyncModal, BusyLabel, useAsyncJob } from "./AsyncModal";
import { Modal } from "./Modal";
import type {
  CreateFromTemplateResult,
  SiteTemplateInfo,
  SiteTemplateKind,
} from "../types";

/** Create a Laravel / Symfony / Statamic / Drupal / Joomla / WooCommerce site in one step. */
export function TemplatePickerModal({
  phpOptions,
  initialKind,
  onClose,
  onCreated,
}: {
  phpOptions: string[];
  initialKind?: SiteTemplateKind;
  onClose: () => void;
  onCreated: (result: CreateFromTemplateResult) => void;
}) {
  const job = useAsyncJob();
  const [kinds, setKinds] = useState<SiteTemplateInfo[]>([]);
  const [kind, setKind] = useState<SiteTemplateKind>(initialKind ?? "laravel");
  const [name, setName] = useState("my-app");
  const [php, setPhp] = useState(phpOptions[0] ?? "");
  const [engine, setEngine] = useState("mariadb");

  useEffect(() => {
    void client.listSiteTemplateKinds().then(setKinds).catch(() => {});
  }, []);

  const selected = kinds.find((k) => k.kind === kind);

  return (
    <AsyncModal
      title="New site from template"
      description="Scaffolds the project with Composer / WP-CLI, creates a database and adds the site."
      busy={job.busy}
      progress={job.progress}
      error={job.error}
      wide
      onClose={() => {
        if (!job.busy) onClose();
      }}
      actions={
        <>
          <button type="button" className="btn btn-install" disabled={job.busy} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={job.busy || !name.trim()}
            onClick={() =>
              void job.run(async () => {
                const result = await client.createSiteFromTemplate({
                  kind,
                  name: name.trim(),
                  phpVersion: php || null,
                  databaseEngine: engine,
                });
                onCreated(result);
              })
            }
          >
            <BusyLabel busy={job.busy} idle="Create site" busyText="Creating…" progress={job.progress} />
          </button>
        </>
      }
    >
      <div className="template-kinds" role="radiogroup" aria-label="Template">
        {(kinds.length ? kinds : [{ kind, label: kind, docroot: "", commands: [] }]).map((k) => (
          <button
            key={k.kind}
            type="button"
            role="radio"
            aria-checked={k.kind === kind}
            className={`chip chip-toggle ${k.kind === kind ? "on" : ""}`}
            onClick={() => setKind(k.kind)}
          >
            {k.label}
          </button>
        ))}
      </div>
      {selected?.commands.length ? (
        <pre className="code-block" data-testid="template-commands">
          {selected.commands.join("\n")}
        </pre>
      ) : null}
      <label className="field">
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <div className="field-row">
        <label className="field">
          PHP
          <select className="native" value={php} onChange={(e) => setPhp(e.target.value)}>
            {phpOptions.length === 0 ? <option value="">Default</option> : null}
            {phpOptions.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          Database
          <select className="native" value={engine} onChange={(e) => setEngine(e.target.value)}>
            <option value="mariadb">MariaDB</option>
            <option value="mysql">MySQL</option>
          </select>
        </label>
      </div>
    </AsyncModal>
  );
}

export function TemplateResultModal({
  result,
  onClose,
}: {
  result: CreateFromTemplateResult;
  onClose: () => void;
}) {
  const titleId = useId();
  return (
    <Modal
      onClose={onClose}
      labelledBy={titleId}
      testId="template-result"
      style={{ maxWidth: 520 }}
      initialFocus="[data-autofocus]"
    >
      <h3 id={titleId}>{result.site.name} created</h3>
      <p className="muted">{result.url}</p>
      <ul className="xdebug-steps">
        {result.steps.map((s) => (
          <li key={s.name} style={{ color: s.ok ? undefined : "var(--danger)" }}>
            <strong>{s.name}</strong>: {s.message}
            {s.skipped ? " (skipped)" : ""}
          </li>
        ))}
      </ul>
      {result.database ? (
        <pre className="code-block">{`DB ${result.database.name}\nuser ${result.database.user}\npass ${result.database.password}\nhost ${result.database.host}:${result.database.port}`}</pre>
      ) : null}
      {result.notes.map((n) => (
        <p key={n} className="hint">
          {n}
        </p>
      ))}
      <div className="modal-actions">
        <button type="button" className="btn btn-primary" data-autofocus onClick={onClose}>
          Open site
        </button>
      </div>
    </Modal>
  );
}
