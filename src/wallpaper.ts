import { WALLPAPER_BACKGROUND, WALLPAPER_URL } from "./style";

/** Rust's identifier for the picture it served, built from its path and modification date. */
const VERSION_HEADER = "X-Wallpaper-Version";
const VERSION_URL = `${WALLPAPER_URL}/version`;
let loadedVersion: string | null = null;
let pending: Promise<HTMLImageElement> | null = null;

/**
 * Makes `WALLPAPER_BACKGROUND.image` current and resolves with it. Called whenever the
 * wallpaper is asked for (opening a capture, picking the swatch, exporting), never per
 * frame: a failure is retried on the next request, and a picture that changed since it was
 * loaded is loaded again. Concurrent requests share one load. Rejects with the reason.
 */
export function requestWallpaper(): Promise<HTMLImageElement> {
  pending ??= refresh().finally(() => {
    pending = null;
  });
  return pending;
}

/** The reason a request failed, as the status line shows it. */
export function wallpaperError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function refresh(): Promise<HTMLImageElement> {
  const current = WALLPAPER_BACKGROUND.image;
  if (current && loadedVersion !== null && (await fetchText(VERSION_URL)) === loadedVersion) return current;
  const response = await fetchChecked(WALLPAPER_URL);
  const version = response.headers.get(VERSION_HEADER);
  // A blob URL is same-origin, so drawing it never taints the exported canvas.
  const url = URL.createObjectURL(await response.blob());
  const image = new Image();
  image.src = url;
  try {
    await image.decode();
  } catch {
    URL.revokeObjectURL(url);
    throw new Error("el formato de la imagen no se puede leer");
  }
  if (current) URL.revokeObjectURL(current.src);
  WALLPAPER_BACKGROUND.image = image;
  loadedVersion = version;
  return image;
}

/** Fetched rather than set as `src`, so Rust's reason reaches the status line on failure. */
async function fetchChecked(url: string): Promise<Response> {
  const response = await fetch(url);
  if (!response.ok) throw new Error((await response.text()) || `error ${response.status}`);
  return response;
}

async function fetchText(url: string): Promise<string> {
  return (await fetchChecked(url)).text();
}

/** Blur radius on the reduced copy; about Xnapper's softness at any output size. */
const BLUR_RADIUS = 24;
const SATURATION = 1.4;
/** Without canvas filters a much smaller copy, scaled up smoothly, stands in for the blur. */
const SUPPORTS_FILTER = "filter" in CanvasRenderingContext2D.prototype;
const BLURRED_SIDE = SUPPORTS_FILTER ? 480 : 24;
const blurred = new WeakMap<HTMLImageElement, HTMLCanvasElement>();

/**
 * The wallpaper blurred once at a small size. Scaling the result up keeps the blur
 * proportional to the output and makes slider drags cheap, since only this copy is filtered.
 */
export function blurredWallpaper(image: HTMLImageElement): HTMLCanvasElement {
  const cached = blurred.get(image);
  if (cached) return cached;
  const factor = BLURRED_SIDE / Math.max(image.naturalWidth, image.naturalHeight);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.naturalWidth * factor));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * factor));
  const ctx = canvas.getContext("2d")!;
  ctx.imageSmoothingQuality = "high";
  let bleed = 0;
  if (SUPPORTS_FILTER) {
    ctx.filter = `blur(${BLUR_RADIUS}px) saturate(${SATURATION})`;
    // Drawn past the edges so the blur never fades into transparency at the borders.
    bleed = BLUR_RADIUS * 2;
  }
  ctx.drawImage(image, -bleed, -bleed, canvas.width + bleed * 2, canvas.height + bleed * 2);
  blurred.set(image, canvas);
  return canvas;
}
