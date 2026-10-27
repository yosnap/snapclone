import { invoke } from "@tauri-apps/api/core";
import { type MenuItem, showMenu } from "./popover";
import type { Zoom } from "./zoom";

/** Movement, in CSS pixels, that turns a press on "Arrástrame" into a drag. */
const DRAG_THRESHOLD = 3;
const ZOOM_CHOICES = [0.5, 1, 1.5, 2];

export type BottomBarHandlers = {
  copy: () => void;
  save: () => void;
  saveAs: () => void;
  /** Writes the export for dragging; resolves false when there is nothing to drag. */
  prepareDrag: () => Promise<boolean>;
  dragFailed: (error: unknown) => void;
  moreItems: () => MenuItem[];
};

/** The bottom bar: drag out, copy, save, save as, zoom and the "Más…" menu. */
export class BottomBar {
  private readonly copyButton = document.querySelector<HTMLButtonElement>("#copy-button")!;
  private readonly saveButton = document.querySelector<HTMLButtonElement>("#save-button")!;
  private readonly saveAsButton = document.querySelector<HTMLButtonElement>("#save-as-button")!;
  private readonly dragButton = document.querySelector<HTMLElement>("#drag-button")!;
  private readonly zoomLabel = document.querySelector<HTMLElement>("#zoom-label")!;

  constructor(
    private readonly zoom: Zoom,
    private readonly handlers: BottomBarHandlers,
  ) {
    this.copyButton.addEventListener("click", handlers.copy);
    this.saveButton.addEventListener("click", handlers.save);
    this.saveAsButton.addEventListener("click", handlers.saveAs);
    const zoomButton = document.querySelector<HTMLButtonElement>("#zoom-button")!;
    zoomButton.addEventListener("click", () => showMenu(zoomButton, this.zoomItems(), "above"));
    const moreButton = document.querySelector<HTMLButtonElement>("#more-button")!;
    moreButton.addEventListener("click", () => showMenu(moreButton, handlers.moreItems(), "above"));
    this.bindDrag();
  }

  setZoomLabel(label: string): void {
    this.zoomLabel.textContent = label;
  }

  /** Export actions wait while there is no capture or sensitive data is being searched. */
  setExportEnabled(enabled: boolean): void {
    this.copyButton.disabled = !enabled;
    this.saveButton.disabled = !enabled;
    this.saveAsButton.disabled = !enabled;
    this.dragButton.setAttribute("aria-disabled", String(!enabled));
  }

  private zoomItems(): MenuItem[] {
    const fit = this.zoom.isFit;
    return [
      { label: "Ajustar", shortcut: "⌘0", checked: fit, action: () => this.zoom.set("fit") },
      "separator",
      ...ZOOM_CHOICES.map((level) => ({
        label: `${level * 100}%`,
        checked: !fit && this.zoom.label === `${level * 100}%`,
        action: () => this.zoom.set(level),
      })),
      "separator",
      { label: "Acercar", shortcut: "⌘+", action: () => this.zoom.zoomIn() },
      { label: "Alejar", shortcut: "⌘−", action: () => this.zoom.zoomOut() },
    ];
  }

  /**
   * A press writes the image file; once it is ready, the next movement with the button still
   * down asks AppKit to start a file drag from that mouse event.
   */
  private bindDrag(): void {
    let press: { x: number; y: number; ready: boolean; started: boolean } | null = null;
    this.dragButton.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || this.dragButton.getAttribute("aria-disabled") === "true") return;
      this.dragButton.setPointerCapture(event.pointerId);
      const current = { x: event.clientX, y: event.clientY, ready: false, started: false };
      press = current;
      this.handlers.prepareDrag().then(
        (ready) => (current.ready = ready),
        (error) => this.handlers.dragFailed(error),
      );
    });
    this.dragButton.addEventListener("pointermove", (event) => {
      if (!press || !press.ready || press.started) return;
      if (Math.hypot(event.clientX - press.x, event.clientY - press.y) < DRAG_THRESHOLD) return;
      press.started = true;
      invoke("drag_export").catch(this.handlers.dragFailed);
    });
    const end = () => (press = null);
    this.dragButton.addEventListener("pointerup", end);
    this.dragButton.addEventListener("pointercancel", end);
  }
}
