import type { TrayciSettings, UsageState, UsageWindow } from "../shared/types";

type WindowState = Pick<UsageWindow, "usedPercent" | "resetsAt"> & {
  notified: Set<number>;
};

export class QuotaNotifier {
  private readonly windows = new Map<string, WindowState>();

  constructor(
    private readonly getSettings: () => TrayciSettings,
    private readonly notify: (title: string, body: string) => void,
  ) {}

  update(state: UsageState, now = Date.now()): void {
    for (const snapshot of Object.values(state.providers)) {
      if (!snapshot || !["ok", "stale"].includes(snapshot.status)) continue;
      for (const window of snapshot.windows) {
        const key = `${snapshot.provider}:${window.id}`;
        const previous = this.windows.get(key);
        if (snapshot.status === "stale") {
          this.windows.set(key, {
            usedPercent: window.usedPercent,
            resetsAt: window.resetsAt,
            notified: new Set(
              ([80, 90, 95] as const).filter(
                (threshold) => window.usedPercent >= threshold,
              ),
            ),
          });
          continue;
        }
        const reset = previous ? this.didReset(previous, window, now) : false;
        const notified = reset
          ? new Set<number>()
          : (previous?.notified ?? new Set<number>());
        const settings = this.getSettings().notifications;

        if (reset && settings.reset)
          this.notify(
            `${snapshot.displayName} quota reset`,
            `${window.label} quota is available again.`,
          );

        const reached = ([80, 90, 95] as const).filter(
          (threshold) =>
            settings[`quota${threshold}`] &&
            window.usedPercent >= threshold &&
            !notified.has(threshold),
        );
        for (const threshold of reached) notified.add(threshold);
        const threshold = reached.at(-1);
        if (threshold)
          this.notify(
            `${snapshot.displayName} quota at ${threshold}%`,
            `${window.label} has used ${Math.round(window.usedPercent)}%.`,
          );

        this.windows.set(key, {
          usedPercent: window.usedPercent,
          resetsAt: window.resetsAt,
          notified,
        });
      }
    }
  }

  private didReset(
    previous: WindowState,
    current: UsageWindow,
    now: number,
  ): boolean {
    return Boolean(
      (previous.resetsAt &&
        current.resetsAt &&
        previous.resetsAt <= now &&
        current.resetsAt > previous.resetsAt) ||
      (current.usedPercent <= 5 && current.usedPercent < previous.usedPercent),
    );
  }
}
