import { useEffect } from "react";

export type ToastKind = "error" | "info" | "success";

export function Toast({
  message,
  kind = "error",
  onClose,
  ms = 6000,
}: {
  message: string;
  kind?: ToastKind;
  onClose: () => void;
  ms?: number;
}) {
  useEffect(() => {
    const t = window.setTimeout(onClose, ms);
    return () => window.clearTimeout(t);
  }, [message, ms, onClose]);

  return (
    <div
      className={`app-toast app-toast-${kind}`}
      role="status"
      onClick={onClose}
    >
      {message}
    </div>
  );
}
