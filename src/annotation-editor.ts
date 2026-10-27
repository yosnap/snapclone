import {
  ANNOTATION_COLORS,
  type Annotation,
  type AnnotationKind,
  type Point,
  TEXT_SIZE,
  boxFromPoints,
  hitTest,
  moved,
} from "./annotations";
import type { Layout, Overlay } from "./render";

export type Tool = "select" | AnnotationKind;

/** Tool shortcuts, matching the letters shown in the toolbar. */
const TOOL_KEYS: Record<string, Tool> = { v: "select", a: "arrow", r: "rect", t: "text", p: "pixelate" };
const MIN_SHAPE_SIZE = 4;
const HISTORY_LIMIT = 100;

type Geometry = { layout: Layout; scale: number };

type Drag =
  | { mode: "move"; start: Point; original: Annotation; before: Annotation[]; changed: boolean }
  | { mode: "create"; start: Point };

/**
 * Owns the annotation list, the active tool and undo/redo. Pointer input on the canvas
 * is converted to capture coordinates using the current layout.
 */
export class AnnotationEditor {
  tool: Tool = "select";
  color = ANNOTATION_COLORS[0];
  private annotations: Annotation[] = [];
  private selectedId: string | null = null;
  private draft: Annotation | null = null;
  private drag: Drag | null = null;
  private undoStack: Annotation[][] = [];
  private redoStack: Annotation[][] = [];

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
    textEntry.addEventListener("keydown", (event) => {
      if (event.key === "Enter") this.commitText();
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
    this.onChange();
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

  deleteSelected(): void {
    if (!this.selectedId) return;
    const id = this.selectedId;
    this.selectedId = null;
    this.commit(this.annotations.filter((item) => item.id !== id));
  }

  /** Handles editor shortcuts; returns true when the event was consumed. */
  handleKey(event: KeyboardEvent): boolean {
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
    const tool = TOOL_KEYS[key];
    if (tool) {
      this.setTool(tool);
      return true;
    }
    return false;
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

  private toCapturePoint(event: PointerEvent): (Point & Geometry) | null {
    const geometry = this.geometry();
    if (!geometry) return null;
    const rect = this.canvas.getBoundingClientRect();
    return {
      ...geometry,
      x: ((event.clientX - rect.left) * this.canvas.width) / rect.width - geometry.layout.x,
      y: ((event.clientY - rect.top) * this.canvas.height) / rect.height - geometry.layout.y,
    };
  }

  private pointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    const point = this.toCapturePoint(event);
    if (!point) return;

    if (this.tool === "text") {
      this.openTextEntry(point, event, point.scale);
      return;
    }

    this.canvas.setPointerCapture(event.pointerId);
    if (this.tool === "select") {
      const hit = hitTest(this.annotations, point, point.scale);
      this.selectedId = hit?.id ?? null;
      this.drag = hit ? { mode: "move", start: point, original: hit, before: this.annotations, changed: false } : null;
      this.onChange();
      return;
    }

    this.drag = { mode: "create", start: point };
    this.draft = this.shapeFrom(point, point);
    this.onChange();
  }

  private pointerMove(event: PointerEvent): void {
    if (!this.drag) return;
    const point = this.toCapturePoint(event);
    if (!point) return;

    if (this.drag.mode === "move") {
      const { original, start } = this.drag;
      const shifted = moved(original, point.x - start.x, point.y - start.y);
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
      return;
    }

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

  private cancelDrag(): void {
    if (this.drag?.mode === "move") this.annotations = this.drag.before;
    this.drag = null;
    this.draft = null;
    this.onChange();
  }

  private shapeFrom(start: Point, end: Point): Annotation {
    const id = this.draft?.id ?? crypto.randomUUID();
    switch (this.tool) {
      case "arrow":
        return { id, kind: "arrow", color: this.color, from: { x: start.x, y: start.y }, to: { x: end.x, y: end.y } };
      case "pixelate":
        return { id, kind: "pixelate", ...boxFromPoints(start, end) };
      default:
        return { id, kind: "rect", color: this.color, ...boxFromPoints(start, end) };
    }
  }

  private isLargeEnough(annotation: Annotation, scale: number): boolean {
    const minimum = MIN_SHAPE_SIZE * scale;
    if (annotation.kind === "arrow") {
      return Math.hypot(annotation.to.x - annotation.from.x, annotation.to.y - annotation.from.y) >= minimum;
    }
    return annotation.kind === "text" || (annotation.width >= minimum && annotation.height >= minimum);
  }

  private textAnchor: Point | null = null;

  /** Places a native input over the canvas, sized to match the rendered text. */
  private openTextEntry(point: Point, event: PointerEvent, scale: number): void {
    event.preventDefault();
    const pixelsPerCssPixel = this.canvas.width / this.canvas.getBoundingClientRect().width;
    this.textAnchor = { x: point.x, y: point.y };
    this.textEntry.value = "";
    this.textEntry.style.left = `${event.clientX}px`;
    this.textEntry.style.top = `${event.clientY}px`;
    this.textEntry.style.color = this.color;
    this.textEntry.style.fontSize = `${(TEXT_SIZE * scale) / pixelsPerCssPixel}px`;
    this.textEntry.hidden = false;
    requestAnimationFrame(() => this.textEntry.focus());
  }

  private commitText(): void {
    const text = this.textEntry.value.trim();
    const anchor = this.textAnchor;
    this.closeTextEntry();
    if (!text || !anchor) return;
    const annotation: Annotation = { id: crypto.randomUUID(), kind: "text", color: this.color, at: anchor, text };
    this.selectedId = annotation.id;
    this.commit([...this.annotations, annotation]);
  }

  private closeTextEntry(): void {
    this.textAnchor = null;
    this.textEntry.hidden = true;
    this.textEntry.value = "";
  }
}
