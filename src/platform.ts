/** Lightweight OS detection for chrome layout (works in Tauri + browser e2e). */

export type AppPlatform = "windows" | "macos" | "linux" | "web";

export function detectPlatform(): AppPlatform {
  const ua = navigator.userAgent || "";
  const plat = navigator.platform || "";
  if (/Windows/i.test(ua) || /^Win/i.test(plat)) return "windows";
  if (/Mac/i.test(ua) || /MacIntel|MacPPC/i.test(plat)) return "macos";
  if (/Linux/i.test(ua) || /Linux/i.test(plat)) return "linux";
  return "web";
}

export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}
