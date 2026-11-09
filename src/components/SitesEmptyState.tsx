export function SitesEmptyState({
  busy = false,
  onAddFolder,
  onCreateWordPress,
  onImport,
}: {
  busy?: boolean;
  onAddFolder: () => void;
  onCreateWordPress: () => void;
  onImport: () => void;
}) {
  const steps = [
    {
      key: "folder",
      title: "Add a folder",
      body: "Link a project you already have, or create an empty PHP site. Subfolders of a parked directory become name.test automatically.",
      action: "Add folder",
      onClick: onAddFolder,
      primary: true,
    },
    {
      key: "wordpress",
      title: "Create WordPress",
      body: "Downloads WordPress and WP-CLI, creates a database, and installs on a .test domain.",
      action: "Create WordPress",
      onClick: onCreateWordPress,
      primary: false,
    },
    {
      key: "import",
      title: "Import a site",
      body: "Restore a backup or clone a template into a new site.",
      action: "Import site",
      onClick: onImport,
      primary: false,
    },
  ];
  return (
    <div className="sites-empty" data-testid="sites-empty">
      <p className="sites-empty-lead">
        Pick one way to add your first site.
      </p>
      <ol className="sites-empty-steps">
        {steps.map((s, i) => (
          <li
            key={s.key}
            className="panel-card sites-empty-step"
            data-testid={`empty-step-${s.key}`}
          >
            <span className="sites-empty-num" aria-hidden>
              {i + 1}
            </span>
            <div className="sites-empty-body">
              <strong>{s.title}</strong>
              <p className="muted">{s.body}</p>
            </div>
            <button
              type="button"
              className={`btn ${s.primary ? "btn-primary" : "btn-install"}`}
              disabled={busy}
              onClick={s.onClick}
            >
              {s.action}
            </button>
          </li>
        ))}
      </ol>
    </div>
  );
}
