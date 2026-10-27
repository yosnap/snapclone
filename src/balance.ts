import type { Box, Point } from "./annotations";

/**
 * "Balance": when a capture sits on a flat backdrop (a window on a plain page, a dialog
 * on a solid fill) the content is re-centred with equal margins on every side, using the
 * backdrop colour to fill the new space. The analysis below is pure so it can be reasoned
 * about without a canvas; `balancedCanvas` does the drawing.
 */
export type Rgba = [number, number, number, number];
export type Pixels = { data: Uint8ClampedArray; width: number; height: number };

export type BalancePlan = {
  color: Rgba;
  /** Content bounding box in the original capture. */
  content: Box;
  margin: number;
  width: number;
  height: number;
  /** How far the content moves; annotations move by the same amount. */
  offset: Point;
};

export type BalanceResult =
  | { kind: "balanced"; plan: BalancePlan }
  | { kind: "not-uniform" }
  | { kind: "no-content" };

/** Per-channel difference (alpha included) still considered "the same colour". */
export const COLOR_TOLERANCE = 12;
/** Share of border pixels that must match the predominant colour. */
export const UNIFORM_SHARE = 0.9;

export function analyzeBalance(pixels: Pixels): BalanceResult {
  const border = dominantBorderColor(pixels);
  if (!border || border.share < UNIFORM_SHARE) return { kind: "not-uniform" };
  const content = contentBox(pixels, border.color);
  if (!content) return { kind: "no-content" };
  return { kind: "balanced", plan: balancePlan(pixels.width, pixels.height, content, border.color) };
}

/** Centres `content` with the average of its four original margins on every side. */
export function balancePlan(width: number, height: number, content: Box, color: Rgba): BalancePlan {
  const left = content.x;
  const top = content.y;
  const right = width - content.x - content.width;
  const bottom = height - content.y - content.height;
  const margin = Math.round((left + top + right + bottom) / 4);
  return {
    color,
    content,
    margin,
    width: content.width + margin * 2,
    height: content.height + margin * 2,
    offset: { x: margin - left, y: margin - top },
  };
}

/**
 * Most frequent colour on the outermost ring of pixels, found by bucketing to 16 levels per
 * channel and averaging the winning bucket, plus the share of ring pixels close to it.
 */
export function dominantBorderColor(pixels: Pixels): { color: Rgba; share: number } | null {
  const ring = ringOffsets(pixels.width, pixels.height);
  if (ring.length === 0) return null;
  const { data } = pixels;
  const buckets = new Map<number, { count: number; sum: Rgba }>();
  for (const offset of ring) {
    const key = ((data[offset] >> 4) << 12) | ((data[offset + 1] >> 4) << 8) | ((data[offset + 2] >> 4) << 4) | (data[offset + 3] >> 4);
    let bucket = buckets.get(key);
    if (!bucket) {
      bucket = { count: 0, sum: [0, 0, 0, 0] };
      buckets.set(key, bucket);
    }
    bucket.count++;
    for (let channel = 0; channel < 4; channel++) bucket.sum[channel] += data[offset + channel];
  }
  let best = { count: 0, sum: [0, 0, 0, 0] as Rgba };
  for (const bucket of buckets.values()) if (bucket.count > best.count) best = bucket;
  const color = best.sum.map((sum) => Math.round(sum / best.count)) as Rgba;
  const matching = ring.filter((offset) => matches(data, offset, color)).length;
  return { color, share: matching / ring.length };
}

/** Bounding box of the pixels that differ from `color`, scanning inwards from each edge. */
export function contentBox(pixels: Pixels, color: Rgba): Box | null {
  const { data, width, height } = pixels;
  const rowHasContent = (y: number, from: number, to: number) => {
    for (let x = from; x < to; x++) if (!matches(data, (y * width + x) * 4, color)) return true;
    return false;
  };
  const columnHasContent = (x: number, from: number, to: number) => {
    for (let y = from; y < to; y++) if (!matches(data, (y * width + x) * 4, color)) return true;
    return false;
  };

  let top = 0;
  while (top < height && !rowHasContent(top, 0, width)) top++;
  if (top === height) return null;
  let bottom = height - 1;
  while (!rowHasContent(bottom, 0, width)) bottom--;
  let left = 0;
  while (!columnHasContent(left, top, bottom + 1)) left++;
  let right = width - 1;
  while (!columnHasContent(right, top, bottom + 1)) right--;
  return { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

function matches(data: Uint8ClampedArray, offset: number, color: Rgba): boolean {
  return (
    Math.abs(data[offset] - color[0]) <= COLOR_TOLERANCE &&
    Math.abs(data[offset + 1] - color[1]) <= COLOR_TOLERANCE &&
    Math.abs(data[offset + 2] - color[2]) <= COLOR_TOLERANCE &&
    Math.abs(data[offset + 3] - color[3]) <= COLOR_TOLERANCE
  );
}

/** Byte offsets of every pixel on the image's outer edge, each corner counted once. */
function ringOffsets(width: number, height: number): number[] {
  if (width <= 0 || height <= 0) return [];
  const offsets: number[] = [];
  const add = (x: number, y: number) => offsets.push((y * width + x) * 4);
  for (let x = 0; x < width; x++) add(x, 0);
  if (height > 1) for (let x = 0; x < width; x++) add(x, height - 1);
  for (let y = 1; y < height - 1; y++) {
    add(0, y);
    if (width > 1) add(width - 1, y);
  }
  return offsets;
}

/** Reads the pixels of an image or canvas; the source must not taint the canvas (CORS). */
export function readPixels(source: CanvasImageSource, width: number, height: number): Pixels {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.drawImage(source, 0, 0);
  return { data: ctx.getImageData(0, 0, width, height).data, width, height };
}

/** Draws the re-centred capture on a backdrop of the detected colour. */
export function balancedCanvas(source: CanvasImageSource, plan: BalancePlan): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = plan.width;
  canvas.height = plan.height;
  const ctx = canvas.getContext("2d")!;
  const [r, g, b, a] = plan.color;
  ctx.fillStyle = `rgba(${r}, ${g}, ${b}, ${a / 255})`;
  ctx.fillRect(0, 0, plan.width, plan.height);
  const { x, y, width, height } = plan.content;
  ctx.drawImage(source, x, y, width, height, plan.margin, plan.margin, width, height);
  return canvas;
}
