import {
  useEffect,
  useRef,
  type CSSProperties,
  type ReactNode,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";

const FOCUSABLE =
  'a[href],button:not([disabled]),textarea:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';

type ModalProps = {
  children: ReactNode;
  /** Called on Esc / backdrop click when not locked. */
  onClose: () => void;
  /** When true, Esc and backdrop click are ignored. */
  busy?: boolean;
  /** `aria-labelledby` pointing at the dialog title element. */
  labelledBy?: string;
  /** Fallback accessible name when no labelledBy. */
  ariaLabel?: string;
  /** Optional test id on the dialog card. */
  testId?: string;
  wide?: boolean;
  /** Extra class on the card. */
  cardClassName?: string;
  style?: CSSProperties;
  /** Prefer focusing this selector inside the card (e.g. `[data-autofocus]`). */
  initialFocus?: string;
};

/**
 * Viewport-centred modal portal. Always mounts under `document.body` so
 * ancestors with `transform` / scroll containers cannot pin the backdrop.
 */
export function Modal({
  children,
  onClose,
  busy = false,
  labelledBy,
  ariaLabel,
  testId,
  wide,
  cardClassName,
  style,
  initialFocus,
}: ModalProps) {
  const cardRef = useRef<HTMLDivElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  const busyRef = useRef(busy);
  onCloseRef.current = onClose;
  busyRef.current = busy;

  useEffect(() => {
    previousFocus.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    document.body.classList.add("modal-open");

    const card = cardRef.current;
    const focusTarget =
      (initialFocus
        ? card?.querySelector<HTMLElement>(initialFocus)
        : null) ??
      card?.querySelector<HTMLElement>("[data-autofocus]") ??
      card?.querySelector<HTMLElement>(FOCUSABLE) ??
      card;
    const t = window.setTimeout(() => focusTarget?.focus(), 0);

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (busyRef.current) return;
        e.preventDefault();
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab" || !cardRef.current) return;
      const nodes = Array.from(
        cardRef.current.querySelectorAll<HTMLElement>(FOCUSABLE),
      ).filter((el) => !el.hasAttribute("disabled") && el.tabIndex !== -1);
      if (!nodes.length) {
        e.preventDefault();
        cardRef.current.focus();
        return;
      }
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey) {
        if (active === first || !cardRef.current.contains(active)) {
          e.preventDefault();
          last.focus();
        }
      } else if (active === last) {
        e.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("keydown", onKeyDown, true);
      document.body.classList.remove("modal-open");
      previousFocus.current?.focus?.();
    };
  }, [initialFocus]);

  const onBackdrop = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (busyRef.current) return;
    if (e.target === e.currentTarget) onCloseRef.current();
  };

  return createPortal(
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={onBackdrop}
      data-testid="modal-backdrop"
    >
      <div
        ref={cardRef}
        className={["modal-card", cardClassName].filter(Boolean).join(" ")}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        aria-label={labelledBy ? undefined : ariaLabel}
        aria-busy={busy || undefined}
        tabIndex={-1}
        data-testid={testId}
        style={{
          ...(wide ? { maxWidth: 560 } : null),
          ...style,
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
