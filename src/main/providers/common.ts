import { constants } from "node:fs";
import { access, chmod, mkdir } from "node:fs/promises";
import { homedir, tmpdir, userInfo } from "node:os";
import { delimiter, join } from "node:path";
import * as pty from "node-pty";

export type FailureKind =
  | "not-installed"
  | "not-authenticated"
  | "network"
  | "timeout"
  | "rate-limited"
  | "rpc-unavailable"
  | "parse"
  | "aborted"
  | "unknown";

export class ProviderError extends Error {
  constructor(
    public readonly kind: FailureKind,
    message: string,
    public readonly retryAt: number | null = null,
  ) {
    super(message);
  }
}

export const clamp = (value: number): number =>
  Math.min(100, Math.max(0, value));

export function asUsedPercent(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return clamp(
    Math.round((value >= 0 && value <= 1 ? value * 100 : value) * 100) / 100,
  );
}

export function toEpochMs(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value < 100_000_000_000 ? value * 1000 : value;
  }
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? null : parsed;
  }
  return null;
}

async function executable(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

export async function resolveExecutable(
  name: string,
  configured: string | null,
): Promise<string | null> {
  if (configured && (await executable(configured))) return configured;
  const candidates = [
    ...(process.env.PATH ?? "")
      .split(delimiter)
      .filter(Boolean)
      .map((dir) => join(dir, name)),
    join(homedir(), ".local", "bin", name),
    join(homedir(), ".npm-global", "bin", name),
    join(homedir(), ".bun", "bin", name),
  ];
  for (const candidate of new Set(candidates)) {
    if (await executable(candidate)) return candidate;
  }
  return null;
}

export async function boundedCwd(): Promise<string> {
  const root = process.env.XDG_RUNTIME_DIR
    ? join(process.env.XDG_RUNTIME_DIR, "trayci")
    : join(tmpdir(), `trayci-${userInfo().uid}`);
  const directory = join(root, "provider-probe");
  await mkdir(directory, { recursive: true, mode: 0o700 });
  await chmod(root, 0o700).catch(() => undefined);
  await chmod(directory, 0o700).catch(() => undefined);
  return directory;
}

type Killable = { kill(): unknown };
const activeChildren = new Set<Killable>();

export function trackChild<T extends Killable>(child: T): () => void {
  activeChildren.add(child);
  return () => activeChildren.delete(child);
}

export function abortAllChildren(): void {
  for (const child of activeChildren) {
    try {
      child.kill();
    } catch {
      // cleanup is best-effort after the tracked process already exited
    }
  }
  activeChildren.clear();
}

export const activeChildCount = (): number => activeChildren.size;

const ESCAPE = String.fromCharCode(27);
const BELL = String.fromCharCode(7);
const OSC_SEQUENCE = new RegExp(
  `${ESCAPE}\\][^${BELL}]*(?:${BELL}|${ESCAPE}\\\\)`,
  "g",
);
const ANSI_SEQUENCE = new RegExp(
  `${ESCAPE}(?:\\[[0-?]*[ -/]*[@-~]|[@-_])`,
  "g",
);

export function stripTerminalCodes(value: string): string {
  return value
    .replace(OSC_SEQUENCE, "")
    .replace(ANSI_SEQUENCE, "")
    .replace(/\r/g, "\n")
    .split("")
    .filter((character) => {
      const code = character.charCodeAt(0);
      return code === 9 || code === 10 || (code >= 32 && code !== 127);
    })
    .join("");
}

export async function runPty(options: {
  executable: string;
  args?: string[];
  input: string;
  timeoutMs: number;
  signal: AbortSignal;
  complete: (output: string) => boolean;
}): Promise<string> {
  if (options.signal.aborted) throw new ProviderError("aborted", "Cancelled");
  const cwd = await boundedCwd();
  return new Promise((resolve, reject) => {
    const terminal = pty.spawn(options.executable, options.args ?? [], {
      name: "xterm-256color",
      cols: 120,
      rows: 40,
      cwd,
      env: { ...process.env, TERM: "xterm-256color" },
    });
    const untrack = trackChild(terminal);
    let output = "";
    let settled = false;
    let completionTimer: NodeJS.Timeout | undefined;

    const finish = (error?: ProviderError): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      clearTimeout(writeTimer);
      clearTimeout(completionTimer);
      options.signal.removeEventListener("abort", abort);
      untrack();
      try {
        terminal.kill();
      } catch {
        // node-pty throws when the child has already exited
      }
      if (error) reject(error);
      else resolve(stripTerminalCodes(output));
    };
    const abort = (): void => finish(new ProviderError("aborted", "Cancelled"));
    const timeout = setTimeout(
      () => finish(new ProviderError("timeout", "CLI probe timed out")),
      options.timeoutMs,
    );
    const writeTimer = setTimeout(
      () => terminal.write(`${options.input}\r`),
      1_000,
    );

    options.signal.addEventListener("abort", abort, { once: true });
    terminal.onData((chunk) => {
      output = `${output}${chunk}`.slice(-1_000_000);
      if (!completionTimer && options.complete(stripTerminalCodes(output))) {
        completionTimer = setTimeout(() => finish(), 600);
      }
    });
    terminal.onExit(({ exitCode }) => {
      if (options.complete(stripTerminalCodes(output))) finish();
      else
        finish(
          new ProviderError(
            "parse",
            `CLI exited before usage was available (${exitCode})`,
          ),
        );
    });
  });
}

export function resetFromText(text: string, now: number): number | null {
  const duration = text.match(
    /(?:resets?\s+in\s+)?(?:(\d+)d)?\s*(?:(\d+)h)?\s*(?:(\d+)m)?/i,
  );
  if (duration && duration[0].trim() && duration.slice(1).some(Boolean)) {
    const minutes =
      Number(duration[1] ?? 0) * 1440 +
      Number(duration[2] ?? 0) * 60 +
      Number(duration[3] ?? 0);
    return now + minutes * 60_000;
  }
  return toEpochMs(text);
}

export function titleCase(value: string): string {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}
