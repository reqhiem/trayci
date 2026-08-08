type Level = "error" | "warn" | "info" | "debug";

let debugEnabled = false;

export function setDebug(enabled: boolean): void {
  debugEnabled = enabled;
}

export function log(level: Level, message: string, data: Record<string, unknown> = {}): void {
  if (level === "debug" && !debugEnabled) return;
  const safe = Object.fromEntries(
    Object.entries(data).filter(
      ([key]) => !/(token|secret|authorization|cookie|credential|password|key)/i.test(key)
    )
  );
  const line = JSON.stringify({ time: new Date().toISOString(), level, message, ...safe });
  (level === "error" ? console.error : level === "warn" ? console.warn : console.log)(line);
}
