import { WALLPAPER_BACKGROUND, WALLPAPER_URL } from "./style";

type State = "idle" | "loading" | "ready" | "failed";
let state: State = "idle";

/**
 * Loads the desktop picture into `WALLPAPER_BACKGROUND` once; `done` gets `null` when it is
 * ready or the reason it is not. A failure is reported once and not retried until the user
 * picks the wallpaper again (`refreshWallpaper`), so renders never loop on it.
 */
export function ensureWallpaper(done: (error: string | null) => void): void {
  if (state !== "idle") return;
  state = "loading";
  fetchWallpaper().then(
    (image) => {
      WALLPAPER_BACKGROUND.image = image;
      state = "ready";
      done(null);
    },
    (error: unknown) => {
      state = "failed";
      done(error instanceof Error ? error.message : String(error));
    },
  );
}

/** Choosing the wallpaper reads it afresh, in case it changed or failed before. */
export function refreshWallpaper(): void {
  if (state !== "loading") state = "idle";
}

/** Fetched rather than set as `src`, so Rust's reason reaches the status line on failure. */
async function fetchWallpaper(): Promise<HTMLImageElement> {
  const response = await fetch(WALLPAPER_URL);
  if (!response.ok) throw new Error((await response.text()) || `error ${response.status}`);
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
  const previous = WALLPAPER_BACKGROUND.image;
  if (previous) URL.revokeObjectURL(previous.src);
  return image;
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
