import {
  app,
  BrowserWindow,
  nativeImage,
  screen,
  Tray,
  type Rectangle,
} from "electron";
import { join } from "node:path";
import { calculatePopoverPosition } from "../shared/presentation";

export class Popover {
  readonly window: BrowserWindow;
  private anchor: Rectangle | null = null;
  private requestedSize = { width: 360, height: 260 };

  constructor() {
    this.window = new BrowserWindow({
      width: 360,
      height: 260,
      minWidth: 340,
      minHeight: 140,
      frame: false,
      transparent: false,
      resizable: false,
      show: false,
      skipTaskbar: true,
      alwaysOnTop: true,
      backgroundColor: "#0b0f14",
      webPreferences: {
        preload: join(import.meta.dirname, "../preload/index.cjs"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
      },
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
      await this.window.loadFile(
        join(import.meta.dirname, "../renderer/index.html"),
      );
    }
  }

  toggle(trayBounds: Rectangle): void {
    if (this.window.isVisible()) {
      this.window.hide();
      return;
    }
    if (!this.anchor) {
      const hasTrayBounds = trayBounds.width > 0 && trayBounds.height > 0;
      const point = screen.getCursorScreenPoint();
      this.anchor = hasTrayBounds
        ? { ...trayBounds }
        : { x: point.x, y: point.y, width: 0, height: 0 };
      this.reposition();
    }
    this.window.show();
    this.window.focus();
  }

  resize(width: number, height: number): void {
    const nextWidth = Math.min(720, Math.max(340, Math.round(width)));
    const nextHeight = Math.min(620, Math.max(140, Math.round(height)));
    if (
      this.requestedSize.width === nextWidth &&
      this.requestedSize.height === nextHeight
    )
      return;
    this.requestedSize = { width: nextWidth, height: nextHeight };
    this.window.setContentSize(nextWidth, nextHeight);
    if (this.anchor) this.reposition();
  }

  private reposition(): void {
    if (!this.anchor) return;
    const point = {
      x: Math.round(this.anchor.x + this.anchor.width / 2),
      y: Math.round(this.anchor.y + this.anchor.height / 2),
    };
    const display = screen.getDisplayNearestPoint(point);
    const position = calculatePopoverPosition(
      this.anchor,
      display.workArea,
      this.window.getBounds(),
    );
    this.window.setPosition(position.x, position.y);
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
