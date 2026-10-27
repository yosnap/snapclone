import { type Annotation, drawAnnotations, drawSelection } from "./annotations";
import { type Background, type Style, ratioValue } from "./style";

export type Capture = { image: HTMLImageElement; scale: number };

const WATERMARK_FONT_SIZE = 12;
const WATERMARK_GAP = 10;

export type Overlay = {
  annotations: Annotation[];
  selected: Annotation | null;
};

/** Where the capture sits inside the composed canvas, in canvas pixels. */
export type Layout = { width: number; height: number; x: number; y: number; radius: number };

export function layout(capture: Capture, style: Style): Layout {
  const { image, scale } = capture;
  const padding = style.padding * scale;
  const fitted = fitRatio(image.naturalWidth + padding * 2, image.naturalHeight + padding * 2, ratioValue(style.ratio));
  const width = Math.round(fitted.width);
  const height = Math.round(fitted.height);
  return {
    width,
    height,
    x: Math.round((width - image.naturalWidth) / 2),
    y: Math.round((height - image.naturalHeight) / 2),
    radius: Math.min(style.radius * scale, image.naturalWidth / 2, image.naturalHeight / 2),
  };
}

/**
 * Draws the final composition at capture resolution. The same canvas is the preview
 * and the export source, so what is shown is exactly what gets copied or saved;
 * exports pass no selection so the dashed outline never leaks into the output.
 */
export function render(canvas: HTMLCanvasElement, capture: Capture, style: Style, overlay: Overlay): void {
  const { image, scale } = capture;
  const { width, height, x, y, radius } = layout(capture, style);

  resize(canvas, width, height);
  const ctx = canvas.getContext("2d")!;
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  paintBackground(ctx, style.background, canvas.width, canvas.height);

  // Drawing the already-rounded capture with a shadow makes the shadow follow its alpha,
  // so transparent pixels (window captures) never reveal a solid backing shape.
  const rounded = roundedCapture(image, radius);
  ctx.save();
  if (style.shadow > 0) {
    ctx.shadowColor = "rgba(0, 0, 0, 0.45)";
    ctx.shadowBlur = style.shadow * scale;
    ctx.shadowOffsetY = style.shadow * scale * 0.4;
  }
  ctx.drawImage(rounded, x, y);
  ctx.restore();

  ctx.save();
  roundedRect(ctx, x, y, image.naturalWidth, image.naturalHeight, radius);
  ctx.clip();
  ctx.translate(x, y);
  drawAnnotations(ctx, image, overlay.annotations, scale);
  ctx.restore();

  if (overlay.selected) {
    ctx.save();
    ctx.translate(x, y);
    drawSelection(ctx, overlay.selected, scale);
    ctx.restore();
  }

  if (style.watermark.trim()) {
    paintWatermark(ctx, style.watermark.trim(), x + image.naturalWidth, y + image.naturalHeight, scale);
  }
}

const roundedLayer = document.createElement("canvas");

function roundedCapture(image: HTMLImageElement, radius: number): HTMLCanvasElement {
  resize(roundedLayer, image.naturalWidth, image.naturalHeight);
  const ctx = roundedLayer.getContext("2d")!;
  ctx.clearRect(0, 0, roundedLayer.width, roundedLayer.height);
  ctx.save();
  roundedRect(ctx, 0, 0, roundedLayer.width, roundedLayer.height, radius);
  ctx.clip();
  ctx.drawImage(image, 0, 0);
  ctx.restore();
  return roundedLayer;
}

/** Assigning width/height reallocates the backing store, so only do it on real changes. */
function resize(canvas: HTMLCanvasElement, width: number, height: number): void {
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
}

/** Grows the shorter side so the canvas matches the ratio; the image never shrinks. */
function fitRatio(width: number, height: number, ratio: number | null) {
  if (ratio === null) return { width, height };
  return width / height < ratio
    ? { width: height * ratio, height }
    : { width, height: width / ratio };
}

function paintBackground(
  ctx: CanvasRenderingContext2D,
  background: Background,
  width: number,
  height: number,
): void {
  switch (background.kind) {
    case "solid":
      ctx.fillStyle = background.color;
      break;
    case "gradient": {
      const fill = cssLinearGradient(ctx, background.angle, width, height);
      background.stops.forEach((stop) => fill.addColorStop(stop.offset, stop.color));
      ctx.fillStyle = fill;
      break;
    }
    case "image":
      drawCover(ctx, background.image, width, height);
      return;
  }
  ctx.fillRect(0, 0, width, height);
}

/** Reproduces CSS `linear-gradient(<angle>deg, …)` geometry on a canvas. */
function cssLinearGradient(ctx: CanvasRenderingContext2D, angle: number, width: number, height: number) {
  const radians = (angle * Math.PI) / 180;
  const dx = Math.sin(radians);
  const dy = -Math.cos(radians);
  const half = (Math.abs(width * dx) + Math.abs(height * dy)) / 2;
  const cx = width / 2;
  const cy = height / 2;
  return ctx.createLinearGradient(cx - dx * half, cy - dy * half, cx + dx * half, cy + dy * half);
}

function drawCover(ctx: CanvasRenderingContext2D, image: HTMLImageElement, width: number, height: number) {
  const scale = Math.max(width / image.naturalWidth, height / image.naturalHeight);
  const drawWidth = image.naturalWidth * scale;
  const drawHeight = image.naturalHeight * scale;
  ctx.drawImage(image, (width - drawWidth) / 2, (height - drawHeight) / 2, drawWidth, drawHeight);
}

function roundedRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
}

/** Right-aligned below the image, like Xnapper; clamped inside the canvas when padding is small. */
function paintWatermark(ctx: CanvasRenderingContext2D, text: string, right: number, bottom: number, scale: number) {
  const fontSize = WATERMARK_FONT_SIZE * scale;
  const baseline = Math.min(bottom + (WATERMARK_GAP + WATERMARK_FONT_SIZE) * scale, ctx.canvas.height - 4 * scale);
  ctx.save();
  ctx.font = `400 ${fontSize}px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
  ctx.textAlign = "right";
  ctx.globalAlpha = 0.8;
  ctx.shadowColor = "#000";
  ctx.shadowBlur = scale;
  ctx.shadowOffsetX = scale;
  ctx.shadowOffsetY = scale;
  ctx.fillStyle = "#fff";
  ctx.fillText(text, right, baseline);
  ctx.restore();
}
