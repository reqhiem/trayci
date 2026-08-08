import { app, BrowserWindow, nativeImage, screen, Tray, type Rectangle } from "electron";
import { join } from "node:path";
import { calculatePopoverPosition } from "../shared/presentation";

export class Popover {
  readonly window: BrowserWindow;

  constructor() {
    this.window = new BrowserWindow({
      width: 390,
      height: 560,
      minWidth: 360,
      minHeight: 420,
      frame: false,
      transparent: false,
      resizable: false,
      show: false,
      skipTaskbar: true,
      alwaysOnTop: true,
      backgroundColor: "#111318",
      webPreferences: {
        preload: join(import.meta.dirname, "../preload/index.cjs"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true
      }
    });
    this.window.on("blur", () => this.window.hide());
    this.window.on("close", (event) => {
      if (!this.window.isDestroyed()) {
        event.preventDefault();
        this.window.hide();
      }
    });
  }

  async load(): Promise<void> {
    if (process.env.ELECTRON_RENDERER_URL) {
      await this.window.loadURL(process.env.ELECTRON_RENDERER_URL);
    } else {
      await this.window.loadFile(join(import.meta.dirname, "../renderer/index.html"));
    }
  }

  toggle(trayBounds: Rectangle): void {
    if (this.window.isVisible()) {
      this.window.hide();
      return;
    }
    const bounds = this.window.getBounds();
    const hasTrayBounds = Boolean(trayBounds.width || trayBounds.height);
    const point = hasTrayBounds
      ? { x: Math.round(trayBounds.x + trayBounds.width / 2), y: Math.round(trayBounds.y + trayBounds.height / 2) }
      : screen.getCursorScreenPoint();
    const display = screen.getDisplayNearestPoint(point);
    const anchor = hasTrayBounds ? trayBounds : { x: point.x, y: point.y, width: 0, height: 0 };
    const position = calculatePopoverPosition(anchor, display.workArea, bounds);
    this.window.setPosition(position.x, position.y);
    this.window.show();
    this.window.focus();
  }
}

function iconPath(): string {
  return !app.isPackaged
    ? join(process.cwd(), "resources", "icons", "32x32.png")
    : join(process.resourcesPath, "resources", "icons", "32x32.png");
}

export class TrayManager {
  readonly tray: Tray;

  constructor(toggle: (bounds: Rectangle) => void) {
    const image = nativeImage.createFromPath(iconPath());
    this.tray = new Tray(image.resize({ width: 22, height: 22 }));
    this.tray.setToolTip("Trayci");
    this.tray.on("click", () => toggle(this.tray.getBounds()));
  }

  setTooltip(value: string): void {
    this.tray.setToolTip(value);
  }

  destroy(): void {
    this.tray.destroy();
  }
}
