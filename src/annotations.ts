/**
 * Annotation model and drawing. Coordinates are capture pixels with the origin at the
 * capture's top-left corner, so annotations stay put when padding or ratio change.
 * Sizes are in points and are multiplied by the capture scale when drawn.
 */
export type Point = { x: number; y: number };
export type Box = { x: number; y: number; width: number; height: number };

type Colored = { id: string; color: string };
export type ArrowAnnotation = Colored & { kind: "arrow"; from: Point; to: Point };
export type RectAnnotation = Colored & Box & { kind: "rect" };
export type TextAnnotation = Colored & { kind: "text"; at: Point; text: string };
export type PixelateAnnotation = Box & { id: string; kind: "pixelate" };
export type Annotation = ArrowAnnotation | RectAnnotation | TextAnnotation | PixelateAnnotation;
export type AnnotationKind = Annotation["kind"];

export const ANNOTATION_COLORS = ["#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7", "#ffffff", "#111827"];

const STROKE_WIDTH = 4;
export const TEXT_SIZE = 28;
const PIXEL_BLOCK = 10;
const HIT_TOLERANCE = 8;
/** Shared by canvas text, the floating text input and the watermark so they always match. */
export const FONT_STACK = '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
export const FONT_WEIGHT = 600;

export function textFont(scale: number): string {
  return `${FONT_WEIGHT} ${TEXT_SIZE * scale}px ${FONT_STACK}`;
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
      return boxFromPoints(annotation.from, annotation.to);
    case "text": {
      measureContext.font = textFont(scale);
      const width = measureContext.measureText(annotation.text).width;
      return { x: annotation.at.x, y: annotation.at.y, width, height: TEXT_SIZE * scale * 1.2 };
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
    const hit =
      annotation.kind === "arrow"
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
      return { ...annotation, from: shift(annotation.from), to: shift(annotation.to) };
    case "text":
      return { ...annotation, at: shift(annotation.at) };
    default:
      return { ...annotation, x: annotation.x + dx, y: annotation.y + dy };
  }
}

/** Expects the context translated to the capture origin and clipped to it. */
export function drawAnnotations(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  annotations: Annotation[],
  scale: number,
): void {
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
      case "text":
        ctx.font = textFont(scale);
        ctx.textBaseline = "top";
        ctx.fillStyle = annotation.color;
        ctx.fillText(annotation.text, annotation.at.x, annotation.at.y + TEXT_SIZE * scale * 0.1);
        break;
      case "pixelate":
        drawPixelate(ctx, image, annotation, scale);
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
  const width = STROKE_WIDTH * scale;
  const angle = Math.atan2(arrow.to.y - arrow.from.y, arrow.to.x - arrow.from.x);
  const length = Math.hypot(arrow.to.x - arrow.from.x, arrow.to.y - arrow.from.y);
  const head = Math.min(width * 4, length);
  const spread = Math.PI / 7;
  // The shaft stops at the head's base so its round cap never pokes through the tip.
  const shaftEnd = {
    x: arrow.to.x - Math.cos(angle) * head * 0.8,
    y: arrow.to.y - Math.sin(angle) * head * 0.8,
  };

  ctx.strokeStyle = arrow.color;
  ctx.fillStyle = arrow.color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(arrow.from.x, arrow.from.y);
  ctx.lineTo(shaftEnd.x, shaftEnd.y);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(arrow.to.x, arrow.to.y);
  ctx.lineTo(arrow.to.x - Math.cos(angle - spread) * head, arrow.to.y - Math.sin(angle - spread) * head);
  ctx.lineTo(arrow.to.x - Math.cos(angle + spread) * head, arrow.to.y - Math.sin(angle + spread) * head);
  ctx.closePath();
  ctx.fill();
}

const pixelLayer = document.createElement("canvas");

/** Samples the original capture only, so pixelation never depends on other annotations. */
function drawPixelate(ctx: CanvasRenderingContext2D, image: HTMLImageElement, box: Box, scale: number): void {
  const x = Math.max(0, Math.floor(box.x));
  const y = Math.max(0, Math.floor(box.y));
  const width = Math.min(image.naturalWidth, Math.ceil(box.x + box.width)) - x;
  const height = Math.min(image.naturalHeight, Math.ceil(box.y + box.height)) - y;
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
