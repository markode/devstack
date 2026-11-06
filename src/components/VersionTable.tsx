import type { InstalledPackage, PackageVersion, ProgressEvent } from "../types";

export function VersionTable({
  versions,
  installed,
  busyKey,
  progress,
  installReady = true,
  onInstall,
  onUninstall,
}: {
  versions: PackageVersion[];
  installed: InstalledPackage[];
  busyKey?: string | null;
  progress?: ProgressEvent | null;
  installReady?: boolean;
  onInstall: (version: string) => void;
  onUninstall: (version: string) => void;
}) {
  const installedSet = new Set(
    installed.flatMap((i) => [i.version, i.channel].filter(Boolean) as string[]),
  );

  return (
    <div className="panel-card version-table">
      <table>
        <thead>
          <tr>
            <th>Version</th>
            <th style={{ width: 160 }}>Status</th>
          </tr>
        </thead>
        <tbody>
          {versions.map((v) => {
            const isInstalled =
              installedSet.has(v.version) || installedSet.has(v.channel);
            const key = `${v.channel}:${v.version}`;
            const isBusy = busyKey === key || busyKey === v.version;
            const showProgress =
              isBusy && progress && progress.version === v.version;

            return (
              <tr key={key}>
                <td>
                  {v.label}
                  {v.lts ? <span className="badge">LTS</span> : null}
                </td>
                <td>
                  <div className="row-actions">
                    {showProgress ? (
                      <span className="progress">
                        {progress.phase}
                        {progress.phase === "downloading" && progress.total
                          ? ` ${Math.round(
                              (100 * progress.downloaded) / progress.total,
                            )}%`
                          : ""}
                      </span>
                    ) : null}
                    {isInstalled ? (
                      <>
                        <span className="status-installed">Installed</span>
                        <button
                          type="button"
                          className="btn btn-danger"
                          disabled={!!busyKey}
                          onClick={() => onUninstall(v.version)}
                        >
                          Uninstall
                        </button>
                      </>
                    ) : (
                      <button
                        type="button"
                        className="btn btn-install"
                        disabled={!!busyKey || !installReady || !v.downloadUrl}
                        title={
                          !installReady
                            ? "Install wiring comes in the next milestone"
                            : !v.downloadUrl
                              ? v.notes ?? "No download for this platform yet"
                              : undefined
                        }
                        onClick={() => onInstall(v.version)}
                      >
                        Install
                      </button>
                    )}
                  </div>
                </td>
              </tr>
            );
          })}
          {versions.length === 0 ? (
            <tr>
              <td colSpan={2} className="muted">
                No versions available.
              </td>
            </tr>
          ) : null}
        </tbody>
      </table>
    </div>
  );
}

export function Hero({
  title,
  description,
  color,
  glyph,
}: {
  title: string;
  description: string;
  color: string;
  glyph: string;
}) {
  return (
    <div className="hero-card">
      <div className="hero-icon" style={{ background: color }}>
        {glyph}
      </div>
      <h1>{title}</h1>
      <p>{description}</p>
    </div>
  );
}
