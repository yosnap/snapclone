import { type Annotation, FONT_STACK, drawAnnotations, drawSelection } from "./annotations";
import { dominantBorderColor, readPixels } from "./balance";
import { type Background, type Style, GRADIENT_PRESETS, outputSize, ratioValue } from "./style";
import { blurredWallpaper } from "./wallpaper";

/** The capture as loaded, or a canvas derived from it (a balanced capture). */
export type CaptureSource = HTMLImageElement | HTMLCanvasElement;
/** `width` and `height` are the source's pixel size, read once so both kinds behave alike. */
export type Capture = { image: CaptureSource; width: number; height: number; scale: number };

export function makeCapture(image: CaptureSource, scale: number): Capture {
  const isImage = image instanceof HTMLImageElement;
  return {
    image,
    width: isImage ? image.naturalWidth : image.width,
    height: isImage ? image.naturalHeight : image.height,
    scale,
  };
}

const WATERMARK_FONT_SIZE = 12;
const WATERMARK_GAP = 10;

export type Overlay = {
  annotations: Annotation[];
  selected: Annotation | null;
};

/**
 * Where the capture sits inside the composed canvas, in canvas pixels. `x`/`y` are the
 * capture's origin; the card is the rounded, shadowed frame around it, `inset` wider.
 */
export type Layout = { width: number; height: number; x: number; y: number; inset: number; radius: number };

export function layout(capture: Capture, style: Style): Layout {
  const { width: imageWidth, height: imageHeight, scale } = capture;
  const padding = style.padding * scale;
  const inset = Math.round(style.inset * scale);
  const cardWidth = imageWidth + inset * 2;
  const cardHeight = imageHeight + inset * 2;
  const fitted = fitRatio(cardWidth + padding * 2, cardHeight + padding * 2, ratioValue(style.ratio));
  const width = Math.round(fitted.width);
  const height = Math.round(fitted.height);
  return {
    width,
    height,
    x: Math.round((width - imageWidth) / 2),
    y: Math.round((height - imageHeight) / 2),
    inset,
    radius: Math.min(style.radius * scale, cardWidth / 2, cardHeight / 2),
  };
}

const edgeColors = new WeakMap<CanvasImageSource, string>();

/**
 * The capture's predominant edge colour, which fills the inset so the frame looks like a
 * continuation of the capture's own background. Computed once per capture.
 */
export function edgeColor(capture: Capture): string {
  const cached = edgeColors.get(capture.image);
  if (cached) return cached;
  const border = dominantBorderColor(readPixels(capture.image, capture.width, capture.height));
  const [red, green, blue, alpha] = border?.color ?? [255, 255, 255, 255];
  const color = `rgba(${red}, ${green}, ${blue}, ${(alpha / 255).toFixed(3)})`;
  edgeColors.set(capture.image, color);
  return color;
}

/**
 * The canvas that is copied or saved. With a size such as "1600x900" the composition,
 * already at that ratio, is resampled to exactly that many pixels.
 */
export function outputCanvas(canvas: HTMLCanvasElement, style: Style): HTMLCanvasElement {
  const size = outputSize(style.ratio);
  if (!size || (size.width === canvas.width && size.height === canvas.height)) return canvas;
  const output = document.createElement("canvas");
  output.width = size.width;
  output.height = size.height;
  const ctx = output.getContext("2d")!;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(canvas, 0, 0, size.width, size.height);
  return output;
}

/**
 * Draws the final composition at capture resolution. The same canvas is the preview
 * and the export source, so what is shown is exactly what gets copied or saved;
 * exports pass no selection so the dashed outline never leaks into the output.
 */
export function render(canvas: HTMLCanvasElement, capture: Capture, style: Style, overlay: Overlay): void {
  const { scale } = capture;
  const frame = layout(capture, style);
  const { x, y, inset, radius } = frame;

  resize(canvas, frame.width, frame.height);
  const ctx = canvas.getContext("2d")!;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(baseLayer(capture, style, frame), 0, 0);

  ctx.save();
  roundedRect(ctx, x - inset, y - inset, capture.width + inset * 2, capture.height + inset * 2, radius);
  ctx.clip();
  ctx.translate(x, y);
  drawAnnotations(ctx, capture, overlay.annotations);
  ctx.restore();

  if (overlay.selected) {
    ctx.save();
    ctx.translate(x, y);
    drawSelection(ctx, overlay.selected, scale);
    ctx.restore();
  }
}

const base = { canvas: document.createElement("canvas"), key: "" };
const objectIds = new WeakMap<object, number>();
let nextObjectId = 0;

function objectId(value: object): number {
  let id = objectIds.get(value);
  if (id === undefined) {
    id = nextObjectId++;
    objectIds.set(value, id);
  }
  return id;
}

/**
 * Background, shadowed capture and watermark only change with the style or the capture,
 * so they are cached and annotation drags only repaint what sits on top.
 */
function baseLayer(capture: Capture, style: Style, frame: Layout): HTMLCanvasElement {
  const key = [
    objectId(capture.image),
    capture.scale,
    objectId(style.background),
    // The wallpaper background keeps its identity while its image loads or is replaced.
    style.background.kind === "wallpaper" && style.background.image ? objectId(style.background.image) : "",
    style.padding,
    style.inset,
    style.insetColor ?? "",
    style.radius,
    style.shadow,
    style.ratio,
    style.showWatermark ? style.watermark : "",
  ].join("|");
  if (key === base.key) return base.canvas;

  const { scale } = capture;
  resize(base.canvas, frame.width, frame.height);
  const ctx = base.canvas.getContext("2d")!;
  ctx.clearRect(0, 0, frame.width, frame.height);
  paintBackground(ctx, style.background, frame.width, frame.height);

  // Drawing the already-rounded card with a shadow makes the shadow follow its alpha,
  // so transparent pixels (window captures) never reveal a solid backing shape.
  const card = roundedCard(capture, frame, frame.inset > 0 ? (style.insetColor ?? edgeColor(capture)) : null);
  ctx.save();
  if (style.shadow > 0) {
    ctx.shadowColor = "rgba(0, 0, 0, 0.45)";
    ctx.shadowBlur = style.shadow * scale;
    ctx.shadowOffsetY = style.shadow * scale * 0.4;
  }
  ctx.drawImage(card, frame.x - frame.inset, frame.y - frame.inset);
  ctx.restore();

  if (style.showWatermark && style.watermark.trim()) {
    const bottom = frame.y + capture.height + frame.inset;
    paintWatermark(ctx, style.watermark.trim(), frame.x + capture.width + frame.inset, bottom, scale);
  }
  base.key = key;
  return base.canvas;
}

const roundedLayer = document.createElement("canvas");

/** The capture inside its card: `fill` (the inset colour) around it, all clipped round. */
function roundedCard(capture: Capture, frame: Layout, fill: string | null): HTMLCanvasElement {
  const { inset, radius } = frame;
  resize(roundedLayer, capture.width + inset * 2, capture.height + inset * 2);
  const ctx = roundedLayer.getContext("2d")!;
  ctx.clearRect(0, 0, roundedLayer.width, roundedLayer.height);
  ctx.save();
  roundedRect(ctx, 0, 0, roundedLayer.width, roundedLayer.height, radius);
  ctx.clip();
  if (fill) {
    ctx.fillStyle = fill;
    ctx.fillRect(0, 0, roundedLayer.width, roundedLayer.height);
  }
  ctx.drawImage(capture.image, inset, inset);
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
      ctx.fillRect(0, 0, width, height);
      paintGlows(ctx, background.glows ?? [], width, height);
      return;
    }
    case "none":
      return;
    case "image":
      drawCover(ctx, background.image, background.image.naturalWidth, background.image.naturalHeight, width, height);
      return;
    case "wallpaper": {
      // Until macOS hands over the picture, or if it never does, the default gradient shows.
      if (!background.image) return paintBackground(ctx, GRADIENT_PRESETS[0], width, height);
      const blurred = blurredWallpaper(background.image);
      drawCover(ctx, blurred, blurred.width, blurred.height, width, height);
      return;
    }
  }
  ctx.fillRect(0, 0, width, height);
}

/** Like CSS `radial-gradient(circle at x y, color, transparent radius)` over the fill. */
function paintGlows(ctx: CanvasRenderingContext2D, glows: NonNullable<Extract<Background, { kind: "gradient" }>["glows"]>, width: number, height: number) {
  const side = Math.max(width, height);
  for (const glow of glows) {
    const x = glow.x * width;
    const y = glow.y * height;
    const fill = ctx.createRadialGradient(x, y, 0, x, y, glow.radius * side);
    fill.addColorStop(0, glow.color);
    fill.addColorStop(1, "rgba(255, 255, 255, 0)");
    ctx.fillStyle = fill;
    ctx.fillRect(0, 0, width, height);
  }
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

function drawCover(
  ctx: CanvasRenderingContext2D,
  image: CanvasImageSource,
  imageWidth: number,
  imageHeight: number,
  width: number,
  height: number,
) {
  const scale = Math.max(width / imageWidth, height / imageHeight);
  const drawWidth = imageWidth * scale;
  const drawHeight = imageHeight * scale;
  ctx.save();
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(image, (width - drawWidth) / 2, (height - drawHeight) / 2, drawWidth, drawHeight);
  ctx.restore();
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
  ctx.font = `400 ${fontSize}px ${FONT_STACK}`;
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
