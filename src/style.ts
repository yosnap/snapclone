export type GradientStop = { offset: number; color: string };
/** A soft radial highlight over a gradient; position and radius are fractions of the canvas. */
export type Glow = { x: number; y: number; radius: number; color: string };

export type GradientBackground = { kind: "gradient"; angle: number; stops: GradientStop[]; glows?: Glow[] };

export type Background =
  | GradientBackground
  | { kind: "solid"; color: string }
  | { kind: "image"; image: HTMLImageElement }
  /** The desktop picture, blurred; `image` is filled in once it has loaded (see wallpaper.ts). */
  | { kind: "wallpaper"; image: HTMLImageElement | null }
  /** "Ninguno": a transparent PNG. */
  | { kind: "none" };

/**
 * All lengths are in points; the renderer multiplies them by the capture scale. `ratio` is
 * "auto", a ratio such as "16:9" or an output size in pixels such as "1600x900".
 * `insetColor` overrides the detected edge colour for the capture shown and is never stored.
 */
export type Style = {
  background: Background;
  padding: number;
  inset: number;
  radius: number;
  shadow: number;
  ratio: string;
  watermark: string;
  showWatermark: boolean;
  insetColor: string | null;
};

export const LIMITS = { padding: 300, inset: 200, radius: 50, shadow: 100 } as const;
export type NumericKey = keyof typeof LIMITS;

export type RatioSpec =
  | { kind: "auto" }
  | { kind: "ratio"; width: number; height: number }
  | { kind: "size"; width: number; height: number };

const SIZE_MIN = 16;
const SIZE_MAX = 8000;
const RATIO_MAX = 100;

/** Parses "auto", "4:3" or "1920x1080"; anything else is null. */
export function parseRatio(value: string): RatioSpec | null {
  const text = value.trim().toLowerCase();
  if (text === "auto") return { kind: "auto" };
  const ratio = /^(\d+(?:\.\d+)?)\s*:\s*(\d+(?:\.\d+)?)$/.exec(text);
  if (ratio) {
    const width = Number(ratio[1]);
    const height = Number(ratio[2]);
    const valid = width > 0 && height > 0 && width <= RATIO_MAX && height <= RATIO_MAX;
    return valid ? { kind: "ratio", width, height } : null;
  }
  const size = /^(\d+)\s*[x×]\s*(\d+)$/.exec(text);
  if (size) {
    const width = Number(size[1]);
    const height = Number(size[2]);
    const valid = [width, height].every((side) => side >= SIZE_MIN && side <= SIZE_MAX);
    return valid ? { kind: "size", width, height } : null;
  }
  return null;
}

/** The canonical text of a parsed ratio, so equal choices compare equal. */
export function ratioText(spec: RatioSpec): string {
  if (spec.kind === "auto") return "auto";
  return spec.kind === "ratio" ? `${spec.width}:${spec.height}` : `${spec.width}x${spec.height}`;
}

export function ratioValue(ratio: string): number | null {
  const spec = parseRatio(ratio);
  return !spec || spec.kind === "auto" ? null : spec.width / spec.height;
}

/** The exact output size in pixels when the ratio is a size, otherwise null. */
export function outputSize(ratio: string): { width: number; height: number } | null {
  const spec = parseRatio(ratio);
  return spec?.kind === "size" ? { width: spec.width, height: spec.height } : null;
}

export type RatioChoice = { label: string; value: string };

export const RATIO_CHOICES: RatioChoice[] = [
  { label: "Auto", value: "auto" },
  { label: "4:3", value: "4:3" },
  { label: "3:2", value: "3:2" },
  { label: "16:9", value: "16:9" },
  { label: "1:1", value: "1:1" },
];

/** Recommended image sizes published by each network for posts and shared links. */
export const SOCIAL_SIZES: RatioChoice[] = [
  { label: "Twitter", value: "1600x900" },
  { label: "Facebook", value: "1200x630" },
  { label: "Instagram", value: "1080x1080" },
  { label: "LinkedIn", value: "1200x627" },
  { label: "Youtube", value: "1280x720" },
  { label: "Pinterest", value: "1000x1500" },
  { label: "Reddit", value: "1200x628" },
  { label: "Snapchat", value: "1080x1920" },
];

/**
 * Every gradient, by index; stored styles refer to them by index, so new ones are only ever
 * appended. The first nine come from the presets embedded in Xnapper (CSS angles, 0deg =
 * upwards); the rest are Snapclone's own.
 */
export const GRADIENT_PRESETS: GradientBackground[] = [
  gradient(50, ["hsla(297, 59%, 34%, 1)", "hsla(345, 74%, 58%, 1)", "hsla(22, 87%, 56%, 1)"]),
  gradient(50, ["#02aab0", "#00cdac"]),
  gradient(50, ["hsla(331, 78%, 69%, 1)", "hsla(238, 82%, 70%, 1)"]),
  gradient(50, ["hsla(31, 90%, 76%, 1)", "hsla(302, 82%, 76%, 1)"]),
  gradient(50, ["hsla(29, 92%, 70%, 1)", "hsla(0, 87%, 73%, 1)"]),
  gradient(160, ["#0093E9", "#80D0C7"]),
  gradient(90, ["#ED8F03", "#FFB75E"]),
  gradient(90, ["#F15F79", "#B24592"]),
  gradient(90, ["#8E54E9", "#4776E6"]),
  // Beach: shallow water.
  gradient(160, ["#16b8a0", "#22c7b4", "#5fd8cc"], [{ x: 0.2, y: 0.15, radius: 0.6, color: "rgba(255, 255, 255, 0.28)" }]),
  // Rose: pink and coral with a cool corner.
  gradient(150, ["#f7a1c4", "#f7797d", "#fbb48c"], [{ x: 0.1, y: 0.95, radius: 0.55, color: "rgba(150, 190, 255, 0.75)" }]),
  // Love: deep violet into magenta.
  gradient(150, ["#4b1d8f", "#9b2bb1", "#d9479a"], [{ x: 0.75, y: 0.3, radius: 0.5, color: "rgba(255, 90, 170, 0.45)" }]),
  // Flower: pale lilac petals.
  gradient(135, ["#d9c2f5", "#f1b7e0", "#b9c8f7"], [{ x: 0.3, y: 0.7, radius: 0.5, color: "rgba(255, 255, 255, 0.5)" }]),
  // Sky: blue to cyan with a pink haze.
  gradient(170, ["#5a8dee", "#77c6f4", "#b6e3f7"], [{ x: 0.85, y: 0.85, radius: 0.55, color: "rgba(244, 170, 220, 0.6)" }]),
];

/** Served by Rust from the main display's desktop picture; the page never names a file. */
export const WALLPAPER_URL = "capture://localhost/wallpaper";
/** A small copy for the swatch, so the grid never loads the full picture. */
const WALLPAPER_THUMBNAIL_URL = `${WALLPAPER_URL}/thumbnail`;

/** The single wallpaper background, so it is recognised by identity like the gradient presets. */
export const WALLPAPER_BACKGROUND: Extract<Background, { kind: "wallpaper" }> = { kind: "wallpaper", image: null };
export const NO_BACKGROUND: Extract<Background, { kind: "none" }> = { kind: "none" };

/** The grid of the panel, in Xnapper's order; "Más…" follows it. */
export const NAMED_BACKGROUNDS: { label: string; background: Background }[] = [
  { label: "Escritorio", background: WALLPAPER_BACKGROUND },
  { label: "Cool", background: GRADIENT_PRESETS[5] },
  { label: "Beach", background: GRADIENT_PRESETS[9] },
  { label: "Violet", background: GRADIENT_PRESETS[8] },
  { label: "Rose", background: GRADIENT_PRESETS[10] },
  { label: "Love", background: GRADIENT_PRESETS[11] },
  { label: "Flower", background: GRADIENT_PRESETS[12] },
  { label: "Sky", background: GRADIENT_PRESETS[13] },
  { label: "Ninguno", background: NO_BACKGROUND },
];

/** Gradients offered under "Más…": the ones without a place in the grid. */
export const EXTRA_GRADIENTS = GRADIENT_PRESETS.filter(
  (preset) => !NAMED_BACKGROUNDS.some((named) => named.background === preset),
);

function gradient(angle: number, colors: string[], glows?: Glow[]): GradientBackground {
  const last = Math.max(1, colors.length - 1);
  return {
    kind: "gradient",
    angle,
    stops: colors.map((color, index) => ({ offset: index / last, color })),
    glows,
  };
}

export function defaultStyle(): Style {
  return {
    background: GRADIENT_PRESETS[5],
    padding: 100,
    inset: 0,
    radius: 20,
    shadow: 25,
    ratio: "auto",
    watermark: "Captura hecha con Snapclone",
    showWatermark: false,
    insetColor: null,
  };
}

export function backgroundPreviewCss(background: Background): string {
  switch (background.kind) {
    case "gradient": {
      const stops = background.stops.map((stop) => `${stop.color} ${stop.offset * 100}%`);
      const glows = (background.glows ?? []).map(
        (glow) => `radial-gradient(circle at ${glow.x * 100}% ${glow.y * 100}%, ${glow.color}, transparent ${glow.radius * 100}%)`,
      );
      return [...glows, `linear-gradient(${background.angle}deg, ${stops.join(", ")})`].join(", ");
    }
    case "solid":
      return background.color;
    case "image":
      return `center / cover url("${background.image.src}")`;
    case "wallpaper":
      return `center / cover url("${WALLPAPER_THUMBNAIL_URL}")`;
    case "none":
      return "repeating-conic-gradient(#d9d9d9 0 25%, #ffffff 0 50%) 0 0 / 10px 10px";
  }
}
