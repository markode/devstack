import type { PortCheckResult } from "./types";

/** Parse `… |DEVSTACK_PORT_BUSY:{json}|` from a start error. */
export function parsePortBusy(message: string): PortCheckResult | null {
  const marker = "|DEVSTACK_PORT_BUSY:";
  const idx = message.indexOf(marker);
  if (idx < 0) return null;
  const jsonPart = message.slice(idx + marker.length).replace(/\|\s*$/, "");
  try {
    const raw = JSON.parse(jsonPart) as {
      serviceId?: string;
      suggested?: Record<string, number>;
      conflicts?: PortCheckResult["conflicts"];
    };
    return {
      serviceId: raw.serviceId ?? "",
      ok: false,
      conflicts: raw.conflicts ?? [],
      message: message.slice(0, idx).trim(),
      suggested: raw.suggested ?? {},
    };
  } catch {
    return null;
  }
}

/** Strip machine markers so the UI never shows raw JSON payloads. */
export function humanizeError(message: string): string {
  const busy = parsePortBusy(message);
  if (busy?.message) return busy.message;
  const idx = message.indexOf("|DEVSTACK_");
  if (idx >= 0) return message.slice(0, idx).trim() || message;
  return message;
}
