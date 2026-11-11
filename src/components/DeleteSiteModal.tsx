import { useId, useState } from "react";
import * as client from "../client";
import { BusyLabel, ModalProgress } from "./AsyncModal";
import { Modal } from "./Modal";

export type DeleteSiteModalProps = {
  siteId: string;
  siteName: string;
  absolutePath: string;
  parked?: boolean;
  databaseName?: string | null;
  databaseUser?: string | null;
  onClose: () => void;
  onDeleted: (warnings: string[]) => void;
};

/** Confirm delete with DevStack / DB / folder options — always portaled. */
export function DeleteSiteModal({
  siteId,
  siteName,
  absolutePath,
  parked = false,
  databaseName,
  databaseUser,
  onClose,
  onDeleted,
}: DeleteSiteModalProps) {
  const titleId = useId();
  const [removeFromDevstack, setRemoveFromDevstack] = useState(true);
  const [dropDatabase, setDropDatabase] = useState(false);
  const [deleteFolder, setDeleteFolder] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deleteResult, setDeleteResult] = useState<{
    warnings: string[];
  } | null>(null);

  const dbName = databaseName ?? null;
  const dbUser = databaseUser ?? dbName;

  const finish = (warnings: string[]) => {
    onDeleted(warnings);
  };

  return (
    <Modal
      onClose={() => {
        if (deleteBusy) return;
        if (deleteResult) {
          finish(deleteResult.warnings);
          return;
        }
        onClose();
      }}
      busy={deleteBusy}
      labelledBy={titleId}
      testId="delete-site-modal"
      initialFocus="[data-autofocus]"
    >
      <h3 id={titleId}>
        {deleteResult ? `${siteName} deleted` : `Delete ${siteName}?`}
      </h3>
      {deleteResult ? (
        <>
          <p className="muted" style={{ marginTop: 0 }}>
            The site was removed from DevStack.
          </p>
          {deleteResult.warnings.length ? (
            <div
              className="error-banner modal-inline-error"
              data-testid="delete-site-warnings"
            >
              {deleteResult.warnings.join(" · ")}
            </div>
          ) : (
            <p className="hint" data-testid="delete-site-ok">
              No warnings.
            </p>
          )}
          <div className="modal-actions">
            <button
              type="button"
              className="btn btn-primary"
              data-autofocus
              data-testid="delete-site-done"
              onClick={() => finish(deleteResult.warnings)}
            >
              Done
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="muted" style={{ marginTop: 0 }}>
            Choose what to remove. Parked folders are only deleted from disk when
            that box is checked.
          </p>
          {deleteBusy ? (
            <ModalProgress
              progress={{
                jobId: "delete-site",
                phase: "delete",
                message: "Removing site…",
                bytes: 0,
                total: null,
                percent: null,
              }}
            />
          ) : null}
          {deleteError ? (
            <div className="error-banner modal-inline-error">{deleteError}</div>
          ) : null}
          <fieldset className="modal-fieldset" disabled={deleteBusy}>
            <label className="modal-check-row">
              <input
                type="checkbox"
                checked={removeFromDevstack}
                data-testid="delete-opt-devstack"
                onChange={(e) => setRemoveFromDevstack(e.target.checked)}
              />
              <span className="modal-check-copy">
                <strong className="modal-check-title">Remove from DevStack</strong>
                <span className="modal-check-desc">
                  vhosts, hosts entries, certs, per-site php.ini and logs under the
                  DevStack data dir
                </span>
              </span>
            </label>
            <label className={`modal-check-row${!dbName ? " is-disabled" : ""}`}>
              <input
                type="checkbox"
                checked={dropDatabase}
                disabled={!dbName}
                data-testid="delete-opt-database"
                onChange={(e) => setDropDatabase(e.target.checked)}
              />
              <span className="modal-check-copy">
                <strong className="modal-check-title">Drop database and DB user</strong>
                <span className="modal-check-desc">
                  {dbName ? (
                    <code className="mono">
                      {dbName} / {dbUser ?? "—"}
                    </code>
                  ) : (
                    "No database recorded for this site"
                  )}
                </span>
              </span>
            </label>
            <label className="modal-check-row">
              <input
                type="checkbox"
                checked={deleteFolder}
                data-testid="delete-opt-folder"
                onChange={(e) => setDeleteFolder(e.target.checked)}
              />
              <span className="modal-check-copy">
                <strong className="modal-check-title">Delete site folder from disk</strong>
                <span className="modal-check-desc">
                  <code className="mono" style={{ wordBreak: "break-all" }}>
                    {absolutePath}
                    {parked ? " (parked)" : ""}
                  </code>
                </span>
              </span>
            </label>
          </fieldset>
          <div className="modal-actions">
            <button
              type="button"
              className="btn btn-install"
              disabled={deleteBusy}
              data-autofocus
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-danger-solid"
              disabled={deleteBusy || !removeFromDevstack}
              data-testid="delete-site-confirm"
              onClick={() => {
                setDeleteBusy(true);
                setDeleteError(null);
                void client
                  .deleteSite(siteId, {
                    removeFromDevstack,
                    dropDatabase: Boolean(dbName) && dropDatabase,
                    deleteFolder,
                  })
                  .then((r) => {
                    setDeleteResult({ warnings: r.warnings ?? [] });
                  })
                  .catch((e) =>
                    setDeleteError(e instanceof Error ? e.message : String(e)),
                  )
                  .finally(() => setDeleteBusy(false));
              }}
            >
              <BusyLabel busy={deleteBusy} idle="Delete" busyText="Deleting…" />
            </button>
          </div>
        </>
      )}
    </Modal>
  );
}
