import {
  ANNOTATION_COLORS,
  type Annotation,
  type ArrowHead,
  FONT_STACK,
  type PixelateAnnotation,
  FONT_WEIGHT,
  type Point,
  TEXT_SIZES,
  boxFromPoints,
  hitTest,
  moved,
} from "./annotations";
import { REDACT_MODES, type RedactCategory, type RedactMode } from "./auto-redact";
import type { Layout, Overlay } from "./render";

export type Tool = "select" | "text" | "arrow" | "redact" | "shape";
export type TextSize = keyof typeof TEXT_SIZES;
export type ShapeKind = "rect" | "oval" | "line";
/** Arrow strokes in points: thin, medium and thick. */
export const ARROW_WIDTHS = [2, 4, 7] as const;

/** What each tool draws; repeating a tool's number key steps through its first option. */
export type ToolOptions = {
  textSize: TextSize;
  arrowHead: ArrowHead;
  arrowWidth: number;
  shape: ShapeKind;
};

export const TEXT_SIZE_ORDER: TextSize[] = ["small", "medium", "large"];
export const ARROW_HEAD_ORDER: ArrowHead[] = ["filled", "open", "double"];
export const SHAPE_ORDER: ShapeKind[] = ["rect", "oval", "line"];

export function defaultToolOptions(): ToolOptions {
  return { textSize: "medium", arrowHead: "filled", arrowWidth: ARROW_WIDTHS[1], shape: "rect" };
}

/** Tool shortcuts: the numbers on the toolbar pills, and the letters of earlier versions. */
const TOOL_KEYS: Record<string, Tool> = {
  "1": "text",
  "2": "arrow",
  "3": "redact",
  "4": "shape",
  v: "select",
  a: "arrow",
  r: "shape",
  t: "text",
  p: "redact",
};

function next<T>(order: readonly T[], value: T): T {
  return order[(order.indexOf(value) + 1) % order.length];
}
const MIN_SHAPE_SIZE = 4;
const HISTORY_LIMIT = 100;

export type Geometry = { layout: Layout; scale: number; imageWidth: number; imageHeight: number };

type Drag =
  | { mode: "move"; start: Point; original: Annotation; before: Annotation[]; changed: boolean }
  | { mode: "create"; start: Point };

/** Where the bitmap is drawn inside the canvas element, honouring `object-fit: contain`. */
type ContentRect = { left: number; top: number; cssPerPixel: number };

/**
 * Owns the annotation list, the active tool and undo/redo. Pointer input on the canvas
 * is converted to capture coordinates using the current layout.
 */
export class AnnotationEditor {
  tool: Tool = "select";
  color = ANNOTATION_COLORS[0];
  options: ToolOptions = defaultToolOptions();
  /** Hears every options change, to remember it; set by the owner. */
  onOptionsChange: (options: ToolOptions) => void = () => {};
  /**
   * How regions are hidden, shared with automatic redaction: the owner keeps the one
   * setting and sets it here. `requestRedactMode` asks the owner to change it.
   */
  redactMode: RedactMode = "pixelate";
  requestRedactMode: (mode: RedactMode) => void = (mode) => {
    this.redactMode = mode;
  };
  private annotations: Annotation[] = [];
  private selectedId: string | null = null;
  private draft: Annotation | null = null;
  private drag: Drag | null = null;
  private undoStack: Annotation[][] = [];
  private redoStack: Annotation[][] = [];
  private textAnchor: Point | null = null;
  private pendingHistoryChanges: ((list: Annotation[]) => Annotation[])[] = [];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly textEntry: HTMLInputElement,
    private readonly geometry: () => Geometry | null,
    private readonly onChange: () => void,
  ) {
    canvas.addEventListener("pointerdown", (event) => this.pointerDown(event));
    canvas.addEventListener("pointermove", (event) => this.pointerMove(event));
    canvas.addEventListener("pointerup", (event) => this.pointerUp(event));
    canvas.addEventListener("pointercancel", () => this.cancelDrag());
    textEntry.style.fontFamily = FONT_STACK;
    textEntry.style.fontWeight = String(FONT_WEIGHT);
    textEntry.addEventListener("keydown", (event) => {
      // Enter also confirms IME and accent-picker candidates; let those finish first.
      if (event.key === "Enter" && !event.isComposing) this.commitText();
      if (event.key === "Escape") this.closeTextEntry();
      event.stopPropagation();
    });
    textEntry.addEventListener("blur", () => this.commitText());
  }

  overlay(withSelection: boolean): Overlay {
    const annotations = this.draft ? [...this.annotations, this.draft] : this.annotations;
    const selected = withSelection ? (this.annotations.find((item) => item.id === this.selectedId) ?? null) : null;
    return { annotations, selected };
  }

  /** Starts over for a new capture; history from the previous one no longer applies. */
  reset(): void {
    this.closeTextEntry();
    this.annotations = [];
    this.selectedId = null;
    this.draft = null;
    this.drag = null;
    this.undoStack = [];
    this.redoStack = [];
    this.pendingHistoryChanges = [];
    this.onChange();
  }

  /** Starts a capture with saved annotations, without history: they are its initial state. */
  load(annotations: Annotation[]): void {
    this.reset();
    this.annotations = annotations;
    this.onChange();
  }

  /** The committed annotations, for saving; drafts in progress are not included. */
  snapshot(): Annotation[] {
    return this.annotations;
  }

  /**
   * Changes the tool options; a selected text or arrow takes the new size or style, as one
   * undoable step.
   */
  setOptions(changes: Partial<ToolOptions>): void {
    this.options = { ...this.options, ...changes };
    this.onOptionsChange(this.options);
    const selected = this.annotations.find((item) => item.id === this.selectedId);
    let updated: Annotation | null = null;
    if (selected?.kind === "text" && changes.textSize) {
      updated = { ...selected, size: TEXT_SIZES[changes.textSize] };
    } else if (selected?.kind === "arrow" && (changes.arrowHead || changes.arrowWidth)) {
      // Only the option that was touched changes: the head keeps its width and vice versa.
      updated = {
        ...selected,
        ...(changes.arrowHead ? { head: changes.arrowHead } : {}),
        ...(changes.arrowWidth ? { width: changes.arrowWidth } : {}),
      };
    }
    if (updated) {
      const replacement = updated;
      this.commit(this.annotations.map((item) => (item.id === replacement.id ? replacement : item)));
    } else {
      this.onChange();
    }
  }

  /** The second press of a tool's key: the next size, style, mode or shape. */
  private cycleOption(tool: Tool): void {
    const { options } = this;
    if (tool === "text") this.setOptions({ textSize: next(TEXT_SIZE_ORDER, options.textSize) });
    else if (tool === "arrow") this.setOptions({ arrowHead: next(ARROW_HEAD_ORDER, options.arrowHead) });
    else if (tool === "redact") {
      this.requestRedactMode(next(REDACT_MODES, this.redactMode));
      this.onChange();
    }
    else if (tool === "shape") this.setOptions({ shape: next(SHAPE_ORDER, options.shape) });
  }

  setTool(tool: Tool): void {
    this.tool = tool;
    if (tool !== "select") this.selectedId = null;
    this.canvas.dataset.tool = tool;
    this.onChange();
  }

  /** Changes the colour for new annotations and recolours the selected one. */
  setColor(color: string): void {
    this.color = color;
    const selected = this.annotations.find((item) => item.id === this.selectedId);
    if (selected && selected.kind !== "pixelate" && selected.color !== color) {
      this.commit(this.annotations.map((item) => (item.id === selected.id ? { ...selected, color } : item)));
    }
  }

  undo(): void {
    const previous = this.undoStack.pop();
    if (!previous) return;
    this.redoStack.push(this.annotations);
    this.annotations = previous;
    this.dropMissingSelection();
  }

  redo(): void {
    const next = this.redoStack.pop();
    if (!next) return;
    this.undoStack.push(this.annotations);
    this.annotations = next;
    this.dropMissingSelection();
  }

  /**
   * Automatic regions behave as part of the capture rather than as edits: they are applied
   * to every history snapshot, so undo never uncovers them and the category checkboxes
   * always describe what is on the image. They sit below user annotations, so arrows and
   * text drawn on top stay visible. During a drag they wait until the drag ends.
   */
  addAutoRedactions(regions: PixelateAnnotation[]): void {
    if (regions.length === 0) return;
    const ids = new Set(regions.map((region) => region.id));
    this.applyToHistory((list) => [...regions, ...list.filter((item) => !ids.has(item.id))]);
  }

  /** The automatic regions of `category` as they are now: moved, or some deleted. */
  autoRedactions(category: RedactCategory): PixelateAnnotation[] {
    return this.annotations.filter(
      (item): item is PixelateAnnotation => item.kind === "pixelate" && item.auto === category,
    );
  }

  /** Hides every automatic region the new way, wherever it is now; nothing is re-added. */
  setAutoRedactionMode(mode: RedactMode): void {
    this.applyToHistory((list) =>
      list.map((item) => {
        if (item.kind !== "pixelate" || !item.auto) return item;
        const { mode: _previous, ...rest } = item;
        return mode === "solid" ? { ...rest, mode } : rest;
      }),
    );
  }

  removeAutoRedactions(category: RedactCategory): void {
    this.applyToHistory((list) => list.filter((item) => !(item.kind === "pixelate" && item.auto === category)));
  }

  /**
   * Moves every annotation, in every history snapshot, when the capture under them moves
   * (balancing re-centres its content). Not undoable: undo must not undo the image change.
   */
  translate(dx: number, dy: number): void {
    if (dx === 0 && dy === 0) return;
    this.applyToHistory((list) => list.map((item) => moved(item, dx, dy)));
  }

  private applyToHistory(change: (list: Annotation[]) => Annotation[]): void {
    if (this.drag) {
      this.pendingHistoryChanges.push(change);
      return;
    }
    this.annotations = change(this.annotations);
    this.undoStack = this.undoStack.map(change);
    this.redoStack = this.redoStack.map(change);
    this.dropMissingSelection();
  }

  private flushPendingHistoryChanges(): void {
    const pending = this.pendingHistoryChanges;
    this.pendingHistoryChanges = [];
    pending.forEach((change) => this.applyToHistory(change));
  }

  deleteSelected(): void {
    if (!this.selectedId) return;
    const id = this.selectedId;
    this.selectedId = null;
    this.commit(this.annotations.filter((item) => item.id !== id));
  }

  /** Handles editor shortcuts; returns true when the event was consumed. */
  handleKey(event: KeyboardEvent): boolean {
    // Mid-drag edits would commit a stale snapshot or change the draft's kind.
    if (this.drag) return false;
    const key = event.key.toLowerCase();
    if (event.metaKey && key === "z") {
      if (event.shiftKey) this.redo();
      else this.undo();
      return true;
    }
    if (event.metaKey || event.ctrlKey || event.altKey) return false;
    if (event.key === "Backspace" || event.key === "Delete") {
      this.deleteSelected();
      return true;
    }
    if (event.key === "Escape" && this.selectedId) {
      this.selectedId = null;
      this.onChange();
      return true;
    }
    if (event.key === "Escape" && this.tool !== "select") {
      this.setTool("select");
      return true;
    }
    const tool = TOOL_KEYS[key];
    if (tool) {
      if (tool === this.tool && /^[1-4]$/.test(key) && !event.repeat) this.cycleOption(tool);
      else this.setTool(tool);
      return true;
    }
    return false;
  }

  /** Whether a pointer event falls on the capture rather than on the background around it. */
  isOverCapture(event: MouseEvent): boolean {
    const geometry = this.geometry();
    return geometry !== null && this.isInside(this.toCapturePoint(event, geometry), geometry);
  }

  /** Keeps the floating text input over its anchor after the layout changes. Call after rendering. */
  syncTextEntry(): void {
    const geometry = this.geometry();
    if (this.textEntry.hidden || !this.textAnchor || !geometry) return;
    const content = this.contentRect();
    const { layout, scale } = geometry;
    this.textEntry.style.left = `${content.left + (layout.x + this.textAnchor.x) * content.cssPerPixel}px`;
    this.textEntry.style.top = `${content.top + (layout.y + this.textAnchor.y) * content.cssPerPixel}px`;
    this.textEntry.style.fontSize = `${TEXT_SIZES[this.options.textSize] * scale * content.cssPerPixel}px`;
  }

  /** Replaces the list as one undoable step. Every mutation goes through here. */
  private commit(next: Annotation[], before: Annotation[] = this.annotations): void {
    this.undoStack.push(before);
    if (this.undoStack.length > HISTORY_LIMIT) this.undoStack.shift();
    this.redoStack = [];
    this.annotations = next;
    this.onChange();
  }

  private dropMissingSelection(): void {
    if (!this.annotations.some((item) => item.id === this.selectedId)) this.selectedId = null;
    this.onChange();
  }

  private contentRect(): ContentRect {
    const rect = this.canvas.getBoundingClientRect();
    const cssPerPixel = Math.min(rect.width / this.canvas.width, rect.height / this.canvas.height);
    return {
      left: rect.left + (rect.width - this.canvas.width * cssPerPixel) / 2,
      top: rect.top + (rect.height - this.canvas.height * cssPerPixel) / 2,
      cssPerPixel,
    };
  }

  private toCapturePoint(event: MouseEvent, geometry: Geometry): Point {
    const content = this.contentRect();
    return {
      x: (event.clientX - content.left) / content.cssPerPixel - geometry.layout.x,
      y: (event.clientY - content.top) / content.cssPerPixel - geometry.layout.y,
    };
  }

  /** Annotations live on the capture; anything placed in the padding would be clipped away. */
  private isInside(point: Point, geometry: Geometry): boolean {
    return point.x >= 0 && point.y >= 0 && point.x <= geometry.imageWidth && point.y <= geometry.imageHeight;
  }

  private clampToCapture(point: Point, geometry: Geometry): Point {
    return {
      x: Math.min(Math.max(point.x, 0), geometry.imageWidth),
      y: Math.min(Math.max(point.y, 0), geometry.imageHeight),
    };
  }

  private pointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    const geometry = this.geometry();
    if (!geometry) return;
    const point = this.toCapturePoint(event, geometry);
    const inside = this.isInside(point, geometry);

    if (this.tool === "text") {
      event.preventDefault();
      if (!this.textEntry.hidden) this.commitText();
      if (inside) this.openTextEntry(point);
      return;
    }

    // Drawing tools always draw; annotations move with the selection tool, or with ⌘ held.
    const hit = inside ? hitTest(this.annotations, point, geometry.scale) : undefined;
    if (this.tool === "select" || (event.metaKey && hit)) {
      this.selectedId = hit?.id ?? null;
      if (hit) {
        this.canvas.setPointerCapture(event.pointerId);
        this.drag = { mode: "move", start: point, original: hit, before: this.annotations, changed: false };
      }
      this.onChange();
      return;
    }

    if (!inside) return;
    this.canvas.setPointerCapture(event.pointerId);
    this.drag = { mode: "create", start: point };
    this.draft = this.shapeFrom(point, point);
    this.onChange();
  }

  private pointerMove(event: PointerEvent): void {
    const geometry = this.geometry();
    if (!this.drag || !geometry) return;
    const point = this.clampToCapture(this.toCapturePoint(event, geometry), geometry);

    if (this.drag.mode === "move") {
      const { original, start } = this.drag;
      const dx = point.x - start.x;
      const dy = point.y - start.y;
      // Sub-pixel jitter during a click must not create an undo step.
      if (!this.drag.changed && Math.hypot(dx, dy) < 1) return;
      const shifted = moved(original, dx, dy);
      this.annotations = this.annotations.map((item) => (item.id === original.id ? shifted : item));
      this.drag.changed = true;
    } else {
      this.draft = this.shapeFrom(this.drag.start, point);
    }
    this.onChange();
  }

  private pointerUp(event: PointerEvent): void {
    const drag = this.drag;
    this.drag = null;
    if (!drag) return;
    if (this.canvas.hasPointerCapture(event.pointerId)) this.canvas.releasePointerCapture(event.pointerId);

    if (drag.mode === "move") {
      if (drag.changed) this.commit(this.annotations, drag.before);
    } else {
      const draft = this.draft;
      this.draft = null;
      const geometry = this.geometry();
      if (draft && geometry && this.isLargeEnough(draft, geometry.scale)) {
        this.selectedId = draft.id;
        this.commit([...this.annotations, draft]);
      } else {
        this.onChange();
      }
    }
    this.flushPendingHistoryChanges();
  }

  private cancelDrag(): void {
    if (this.drag?.mode === "move") this.annotations = this.drag.before;
    this.drag = null;
    this.draft = null;
    this.onChange();
    this.flushPendingHistoryChanges();
  }

  private shapeFrom(start: Point, end: Point): Annotation {
    const id = this.draft?.id ?? crypto.randomUUID();
    const { arrowHead, arrowWidth, shape } = this.options;
    const { redactMode } = this;
    if (this.tool === "arrow") {
      return { id, kind: "arrow", color: this.color, from: { ...start }, to: { ...end }, head: arrowHead, width: arrowWidth };
    }
    if (this.tool === "redact") {
      return { id, kind: "pixelate", ...boxFromPoints(start, end), ...(redactMode === "solid" ? { mode: redactMode } : {}) };
    }
    if (shape === "line") return { id, kind: "line", color: this.color, from: { ...start }, to: { ...end } };
    return { id, kind: shape, color: this.color, ...boxFromPoints(start, end) };
  }

  /** Drafts are only arrows and boxes; text is created through the input instead. */
  private isLargeEnough(annotation: Annotation, scale: number): boolean {
    const minimum = MIN_SHAPE_SIZE * scale;
    if (annotation.kind === "arrow" || annotation.kind === "line") {
      return Math.hypot(annotation.to.x - annotation.from.x, annotation.to.y - annotation.from.y) >= minimum;
    }
    return "width" in annotation && annotation.width >= minimum && annotation.height >= minimum;
  }

  private openTextEntry(point: Point): void {
    this.textAnchor = point;
    this.textEntry.value = "";
    this.textEntry.style.color = this.color;
    this.textEntry.hidden = false;
    this.syncTextEntry();
    requestAnimationFrame(() => this.textEntry.focus());
  }

  private commitText(): void {
    const text = this.textEntry.value.trim();
    const anchor = this.textAnchor;
    this.closeTextEntry();
    if (!text || !anchor) return;
    const size = TEXT_SIZES[this.options.textSize];
    const annotation: Annotation = { id: crypto.randomUUID(), kind: "text", color: this.color, at: anchor, text, size };
    this.selectedId = annotation.id;
    this.commit([...this.annotations, annotation]);
  }

  private closeTextEntry(): void {
    this.textAnchor = null;
    this.textEntry.hidden = true;
    this.textEntry.value = "";
  }
}
