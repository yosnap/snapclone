/**
 * Preview zoom. 100 % shows the composition at its size in points (one capture point per
 * CSS pixel); "Ajustar" fits it in the canvas area with a margin, never above 100 %.
 * Only the canvas element's CSS size changes: the bitmap stays at capture resolution, so
 * the annotation editor keeps converting pointer positions from the element's box.
 */
export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 4;
const STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.25, 1.5, 2, 3, 4];
/** Space kept around the preview when it is fitted, like Xnapper's canvas. */
export const FIT_MARGIN = 18;

export type ZoomLevel = "fit" | number;

export class Zoom {
  private level: ZoomLevel = "fit";
  private shown = 1;

  constructor(
    private readonly area: HTMLElement,
    private readonly canvas: HTMLCanvasElement,
    /** Capture pixels per point of the capture shown, or null without one. */
    private readonly scale: () => number | null,
    private readonly onChange: (label: string) => void,
  ) {
    new ResizeObserver(() => this.update()).observe(area);
  }

  get label(): string {
    return `${Math.round(this.shown * 100)}%`;
  }

  get isFit(): boolean {
    return this.level === "fit";
  }

  set(level: ZoomLevel): void {
    this.level = level === "fit" ? "fit" : clamp(level);
    this.update();
  }

  zoomIn(): void {
    this.set(STEPS.find((step) => step > this.shown + 0.001) ?? ZOOM_MAX);
  }

  zoomOut(): void {
    this.set([...STEPS].reverse().find((step) => step < this.shown - 0.001) ?? ZOOM_MIN);
  }

  /** Call after every render: the composition size changes with padding, ratio and capture. */
  update(): void {
    const scale = this.scale();
    if (!scale || this.canvas.hidden) return;
    const width = this.canvas.width / scale;
    const height = this.canvas.height / scale;
    if (width <= 0 || height <= 0) return;
    this.shown = this.level === "fit" ? this.fitLevel(width, height) : this.level;
    this.canvas.style.width = `${Math.round(width * this.shown)}px`;
    this.canvas.style.height = `${Math.round(height * this.shown)}px`;
    this.area.classList.toggle("zoomed", this.level !== "fit");
    this.onChange(this.label);
  }

  private fitLevel(width: number, height: number): number {
    const availableWidth = this.area.clientWidth - FIT_MARGIN * 2;
    const availableHeight = this.area.clientHeight - FIT_MARGIN * 2;
    if (availableWidth <= 0 || availableHeight <= 0) return ZOOM_MIN;
    return Math.max(ZOOM_MIN, Math.min(1, availableWidth / width, availableHeight / height));
  }
}

function clamp(level: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, level));
}
