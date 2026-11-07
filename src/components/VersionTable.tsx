import type { InstalledPackage, PackageVersion, ProgressEvent } from "../types";

export function VersionTable({
  versions,
  installed,
  busyKey,
  progress,
  installReady = true,
  onInstall,
  onUninstall,
  onUnavailable,
}: {
  versions: PackageVersion[];
  installed: InstalledPackage[];
  busyKey?: string | null;
  progress?: ProgressEvent | null;
  installReady?: boolean;
  onInstall: (version: string) => void;
  onUninstall: (version: string) => void;
  /** Called when the user clicks Install but the row has no usable download. */
  onUnavailable?: (message: string) => void;
}) {
  const installedSet = new Set(
    installed.flatMap((i) => [i.version, i.channel].filter(Boolean) as string[]),
  );

  const unavailableCount = versions.filter(
    (v) =>
      !installedSet.has(v.version) &&
      !installedSet.has(v.channel) &&
      (!installReady || !v.downloadUrl),
  ).length;

  return (
    <div className="panel-card version-table">
      {unavailableCount > 0 ? (
        <div className="error-banner" style={{ margin: "12px 12px 0" }}>
          {unavailableCount} version
          {unavailableCount === 1 ? "" : "s"} cannot be installed on this
          platform
          {!installReady
            ? " (package install is not enabled)."
            : " (no download URL)."}{" "}
          Click Install for details.
        </div>
      ) : null}
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
            const canInstall = installReady && !!v.downloadUrl;
            const unavailableReason = !installReady
              ? "Install is not enabled for this package yet."
              : !v.downloadUrl
                ? (v.notes ??
                  "No download is available for this OS/architecture.")
                : null;

            return (
              <tr key={key}>
                <td>
                  {v.label}
                  {v.lts ? <span className="badge">LTS</span> : null}
                  {!isInstalled && unavailableReason ? (
                    <div className="muted" style={{ fontSize: 11, marginTop: 4 }}>
                      {unavailableReason}
                    </div>
                  ) : null}
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
                        disabled={!!busyKey}
                        title={unavailableReason ?? undefined}
                        onClick={() => {
                          if (!canInstall) {
                            onUnavailable?.(
                              unavailableReason ??
                                "No download is available for this platform.",
                            );
                            return;
                          }
                          onInstall(v.version);
                        }}
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
