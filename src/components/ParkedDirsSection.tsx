import { useCallback, useEffect, useState } from "react";
import * as client from "../client";

/** Parked directories: every subfolder becomes `<name>.test`. */
export function ParkedDirsSection({
  pickFolder,
  onNote,
  onError,
}: {
  pickFolder: () => Promise<string | null>;
  onNote?: (msg: string) => void;
  onError?: (msg: string) => void;
}) {
  const [dirs, setDirs] = useState<string[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);

  const fail = useCallback(
    (e: unknown) => onError?.(e instanceof Error ? e.message : String(e)),
    [onError],
  );

  useEffect(() => {
    void client
      .getSitesSettings()
      .then((s) => setDirs(s.parkedDirs))
      .catch(fail)
      .finally(() => setLoaded(true));
  }, [fail]);

  const save = async (next: string[]) => {
    setBusy(true);
    try {
      const saved = await client.updateSitesSettings({ parkedDirs: next });
      setDirs(saved.parkedDirs);
      const r = await client.syncParkedSites();
      onNote?.(
        r.added > 0
          ? `Parked sites synced: ${r.added} new site${r.added === 1 ? "" : "s"}`
          : "Parked directories saved",
      );
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const add = async () => {
    try {
      const picked = await pickFolder();
      if (!picked || dirs.includes(picked)) return;
      await save([...dirs, picked]);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <>
      <div className="section-head" style={{ marginTop: 22 }}>
        <h2>Parked directories</h2>
        <p>
          Each subfolder of a parked directory becomes{" "}
          <code>name.test</code> automatically — no manual setup per project.
        </p>
      </div>
      <div className="panel-card settings-list" data-testid="parked-dirs">
        {loaded && dirs.length === 0 ? (
          <div className="settings-row">
            <label>
              No parked directories
              <span className="hint">
                e.g. park <code>~/Sites</code>, then <code>~/Sites/blog</code>{" "}
                serves at <code>blog.test</code>.
              </span>
            </label>
          </div>
        ) : null}
        {dirs.map((d) => (
          <div className="settings-row" key={d} data-testid="parked-dir">
            <label>
              <span className="parked-path">{d}</span>
            </label>
            <button
              type="button"
              className="btn btn-danger"
              disabled={busy}
              onClick={() => void save(dirs.filter((x) => x !== d))}
            >
              Remove
            </button>
          </div>
        ))}
        <div className="settings-row">
          <label>
            Add directory
            <span className="hint">Pick a folder that holds your projects</span>
          </label>
          <div className="row-actions">
            <button
              type="button"
              className="btn btn-install"
              disabled={busy || dirs.length === 0}
              title="Rescan parked directories for new subfolders"
              onClick={() => void save(dirs)}
            >
              Rescan
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => void add()}
            >
              Add directory…
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
