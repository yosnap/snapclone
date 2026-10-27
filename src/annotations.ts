import type { RedactCategory, RedactMode } from "./auto-redact";
import type { Capture } from "./render";

/**
 * Annotation model and drawing. Coordinates are capture pixels with the origin at the
 * capture's top-left corner, so annotations stay put when padding or ratio change.
 * Sizes are in points and are multiplied by the capture scale when drawn.
 */
export type Point = { x: number; y: number };
export type Box = { x: number; y: number; width: number; height: number };

type Colored = { id: string; color: string };
export type ArrowHead = "filled" | "open" | "double";
/** `width` is the stroke in points; `head` defaults to "filled". */
export type ArrowAnnotation = Colored & { kind: "arrow"; from: Point; to: Point; width?: number; head?: ArrowHead };
export type RectAnnotation = Colored & Box & { kind: "rect" };
export type OvalAnnotation = Colored & Box & { kind: "oval" };
export type LineAnnotation = Colored & { kind: "line"; from: Point; to: Point };
/** `size` is the font size in points. */
export type TextAnnotation = Colored & { kind: "text"; at: Point; text: string; size?: number };
/**
 * `auto` marks regions found by automatic redaction, so a category can be toggled off later.
 * `mode` "solid" covers the region with a block instead of pixelating it.
 */
export type PixelateAnnotation = Box & { id: string; kind: "pixelate"; auto?: RedactCategory; mode?: RedactMode };
export type Annotation =
  | ArrowAnnotation
  | RectAnnotation
  | OvalAnnotation
  | LineAnnotation
  | TextAnnotation
  | PixelateAnnotation;
export type AnnotationKind = Annotation["kind"];

/** The first one, the system red, is the default, as in Xnapper. */
export const ANNOTATION_COLORS = ["#ff3b30", "#ff9500", "#ffcc00", "#34c759", "#007aff", "#af52de", "#ffffff", "#000000"];

const STROKE_WIDTH = 4;
export const ARROW_WIDTH = 4;
/** Text sizes in points, as Xnapper's small, medium and large. */
export const TEXT_SIZES = { small: 16, medium: 32, large: 56 } as const;
export const TEXT_SIZE = TEXT_SIZES.medium;
const PIXEL_BLOCK = 10;
const REDACT_COLOR = "#000000";
const HIT_TOLERANCE = 8;
/** Shared by canvas text, the floating text input and the watermark so they always match. */
export const FONT_STACK = '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
export const FONT_WEIGHT = 600;

export function textFont(scale: number, size: number = TEXT_SIZE): string {
  return `${FONT_WEIGHT} ${size * scale}px ${FONT_STACK}`;
}

/** Arrows and lines are defined by their two ends rather than by a box. */
function isSegment(annotation: Annotation): annotation is ArrowAnnotation | LineAnnotation {
  return annotation.kind === "arrow" || annotation.kind === "line";
}

export function boxFromPoints(a: Point, b: Point): Box {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

const measureContext = document.createElement("canvas").getContext("2d")!;

export function bounds(annotation: Annotation, scale: number): Box {
  switch (annotation.kind) {
    case "arrow":
    case "line":
      return boxFromPoints(annotation.from, annotation.to);
    case "text": {
      const size = annotation.size ?? TEXT_SIZE;
      measureContext.font = textFont(scale, size);
      const width = measureContext.measureText(annotation.text).width;
      return { x: annotation.at.x, y: annotation.at.y, width, height: size * scale * 1.2 };
    }
    default:
      return { x: annotation.x, y: annotation.y, width: annotation.width, height: annotation.height };
  }
}

/** Topmost annotation under the point, or undefined. */
export function hitTest(annotations: Annotation[], point: Point, scale: number): Annotation | undefined {
  const tolerance = HIT_TOLERANCE * scale;
  for (let index = annotations.length - 1; index >= 0; index--) {
    const annotation = annotations[index];
    const hit = isSegment(annotation)
      ? distanceToSegment(point, annotation.from, annotation.to) <= tolerance
      : contains(bounds(annotation, scale), point, tolerance);
    if (hit) return annotation;
  }
  return undefined;
}

export function moved(annotation: Annotation, dx: number, dy: number): Annotation {
  const shift = (point: Point) => ({ x: point.x + dx, y: point.y + dy });
  switch (annotation.kind) {
    case "arrow":
    case "line":
      return { ...annotation, from: shift(annotation.from), to: shift(annotation.to) };
    case "text":
      return { ...annotation, at: shift(annotation.at) };
    default:
      return { ...annotation, x: annotation.x + dx, y: annotation.y + dy };
  }
}

/** Expects the context translated to the capture origin and clipped to it. */
export function drawAnnotations(ctx: CanvasRenderingContext2D, capture: Capture, annotations: Annotation[]): void {
  const { scale } = capture;
  for (const annotation of annotations) {
    switch (annotation.kind) {
      case "arrow":
        drawArrow(ctx, annotation, scale);
        break;
      case "rect":
        ctx.lineWidth = STROKE_WIDTH * scale;
        ctx.strokeStyle = annotation.color;
        ctx.lineJoin = "round";
        ctx.strokeRect(annotation.x, annotation.y, annotation.width, annotation.height);
        break;
      case "oval":
        ctx.lineWidth = STROKE_WIDTH * scale;
        ctx.strokeStyle = annotation.color;
        ctx.beginPath();
        ctx.ellipse(
          annotation.x + annotation.width / 2,
          annotation.y + annotation.height / 2,
          annotation.width / 2,
          annotation.height / 2,
          0,
          0,
          Math.PI * 2,
        );
        ctx.stroke();
        break;
      case "line":
        ctx.lineWidth = STROKE_WIDTH * scale;
        ctx.strokeStyle = annotation.color;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.moveTo(annotation.from.x, annotation.from.y);
        ctx.lineTo(annotation.to.x, annotation.to.y);
        ctx.stroke();
        break;
      case "text": {
        const size = annotation.size ?? TEXT_SIZE;
        ctx.font = textFont(scale, size);
        ctx.textBaseline = "top";
        ctx.fillStyle = annotation.color;
        ctx.fillText(annotation.text, annotation.at.x, annotation.at.y + size * scale * 0.1);
        break;
      }
      case "pixelate":
        if (annotation.mode === "solid") {
          ctx.fillStyle = REDACT_COLOR;
          ctx.fillRect(annotation.x, annotation.y, annotation.width, annotation.height);
        } else {
          drawPixelate(ctx, capture, annotation);
        }
        break;
    }
  }
}

export function drawSelection(ctx: CanvasRenderingContext2D, annotation: Annotation, scale: number): void {
  const box = bounds(annotation, scale);
  const margin = 4 * scale;
  ctx.save();
  ctx.lineWidth = scale;
  ctx.setLineDash([4 * scale, 3 * scale]);
  ctx.strokeStyle = "#ffffff";
  ctx.strokeRect(box.x - margin, box.y - margin, box.width + margin * 2, box.height + margin * 2);
  ctx.lineDashOffset = 4 * scale;
  ctx.strokeStyle = "#111827";
  ctx.strokeRect(box.x - margin, box.y - margin, box.width + margin * 2, box.height + margin * 2);
  ctx.restore();
}

function drawArrow(ctx: CanvasRenderingContext2D, arrow: ArrowAnnotation, scale: number): void {
  const width = (arrow.width ?? ARROW_WIDTH) * scale;
  const head = arrow.head ?? "filled";
  const angle = Math.atan2(arrow.to.y - arrow.from.y, arrow.to.x - arrow.from.x);
  const length = Math.hypot(arrow.to.x - arrow.from.x, arrow.to.y - arrow.from.y);
  const headLength = Math.min(Math.max(width * 4, 12 * scale), head === "double" ? length / 2 : length);
  // A filled head covers the shaft's end, so the shaft stops at its base and its round cap
  // never pokes through the tip.
  const inset = head === "open" ? 0 : headLength * 0.8;
  const from = head === "double" ? along(arrow.from, angle, inset) : arrow.from;

  ctx.strokeStyle = arrow.color;
  ctx.fillStyle = arrow.color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  const shaftEnd = along(arrow.to, angle + Math.PI, inset);
  ctx.lineTo(shaftEnd.x, shaftEnd.y);
  ctx.stroke();

  drawHead(ctx, arrow.to, angle, headLength, head === "open");
  if (head === "double") drawHead(ctx, arrow.from, angle + Math.PI, headLength, false);
}

function along(point: Point, angle: number, distance: number): Point {
  return { x: point.x + Math.cos(angle) * distance, y: point.y + Math.sin(angle) * distance };
}

function drawHead(ctx: CanvasRenderingContext2D, tip: Point, angle: number, length: number, open: boolean): void {
  const spread = Math.PI / 7;
  const left = along(tip, angle + Math.PI - spread, length);
  const right = along(tip, angle + Math.PI + spread, length);
  ctx.beginPath();
  ctx.moveTo(left.x, left.y);
  ctx.lineTo(tip.x, tip.y);
  ctx.lineTo(right.x, right.y);
  if (open) {
    ctx.stroke();
  } else {
    ctx.closePath();
    ctx.fill();
  }
}

const pixelLayer = document.createElement("canvas");

/** Samples the original capture only, so pixelation never depends on other annotations. */
function drawPixelate(ctx: CanvasRenderingContext2D, capture: Capture, box: Box): void {
  const { image, scale } = capture;
  const x = Math.max(0, Math.floor(box.x));
  const y = Math.max(0, Math.floor(box.y));
  const width = Math.min(capture.width, Math.ceil(box.x + box.width)) - x;
  const height = Math.min(capture.height, Math.ceil(box.y + box.height)) - y;
  if (width <= 0 || height <= 0) return;

  const block = Math.max(2, Math.round(PIXEL_BLOCK * scale));
  const columns = Math.max(1, Math.ceil(width / block));
  const rows = Math.max(1, Math.ceil(height / block));
  if (pixelLayer.width !== columns) pixelLayer.width = columns;
  if (pixelLayer.height !== rows) pixelLayer.height = rows;
  const pixels = pixelLayer.getContext("2d")!;
  pixels.imageSmoothingEnabled = true;
  pixels.drawImage(image, x, y, width, height, 0, 0, columns, rows);

  ctx.save();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(pixelLayer, 0, 0, columns, rows, x, y, width, height);
  ctx.restore();
}

function contains(box: Box, point: Point, tolerance: number): boolean {
  return (
    point.x >= box.x - tolerance &&
    point.x <= box.x + box.width + tolerance &&
    point.y >= box.y - tolerance &&
    point.y <= box.y + box.height + tolerance
  );
}

function distanceToSegment(point: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSquared = dx * dx + dy * dy;
  const t = lengthSquared === 0 ? 0 : Math.max(0, Math.min(1, ((point.x - a.x) * dx + (point.y - a.y) * dy) / lengthSquared));
  return Math.hypot(point.x - (a.x + t * dx), point.y - (a.y + t * dy));
}
