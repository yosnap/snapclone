import {
  type Background,
  type Ratio,
  type Style,
  GRADIENT_PRESETS,
  RATIOS,
  WALLPAPER_BACKGROUND,
  defaultStyle,
} from "./style";

/**
 * Serializable form of `Style`, shared by the settings, the presets and the history.
 * Gradients are stored by preset index and solid colours by value. A background image
 * picked with `<input type=file>` only lives as a blob URL in this session (the file input
 * exposes no path on disk), so it is not persisted: a style saved while one was selected
 * comes back with the default gradient. The desktop picture is read again from macOS, so it
 * is stored by kind alone and survives a restart.
 */
export type StoredBackground =
  | { kind: "gradient"; index: number }
  | { kind: "solid"; color: string }
  | { kind: "wallpaper" };
export type StoredStyle = Omit<Style, "background"> & { background: StoredBackground };

/** Same ranges as the sidebar sliders, so stored values always fit the controls. */
const LIMITS = { padding: 200, radius: 40, shadow: 100 } as const;
const WATERMARK_MAX_LENGTH = 80;
const HEX_COLOR = /^#[0-9a-f]{6}$/i;

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && HEX_COLOR.test(value);
}

export function toStoredStyle(style: Style): StoredStyle {
  const { background, ...rest } = style;
  return { ...rest, background: toStoredBackground(background) };
}

function toStoredBackground(background: Background): StoredBackground {
  if (background.kind === "solid") return { kind: "solid", color: background.color };
  if (background.kind === "wallpaper") return { kind: "wallpaper" };
  const index = background.kind === "gradient" ? GRADIENT_PRESETS.indexOf(background) : -1;
  return index >= 0 ? { kind: "gradient", index } : toStoredBackground(defaultStyle().background);
}

/** Mutates `style` in place, as the controls and the renderer hold on to that object. */
export function applyStoredStyle(style: Style, stored: StoredStyle): void {
  const { background, ...rest } = stored;
  Object.assign(style, rest);
  switch (background.kind) {
    case "solid":
      style.background = { kind: "solid", color: background.color };
      break;
    case "wallpaper":
      style.background = WALLPAPER_BACKGROUND;
      break;
    case "gradient":
      style.background = GRADIENT_PRESETS[background.index];
  }
}

/**
 * Validates untrusted JSON (settings file, presets, history documents). Missing or invalid
 * fields fall back to the defaults instead of discarding the whole style.
 */
export function parseStoredStyle(value: unknown): StoredStyle | null {
  if (!isRecord(value)) return null;
  const fallback = toStoredStyle(defaultStyle());
  return {
    background: parseBackground(value.background) ?? fallback.background,
    padding: clampNumber(value.padding, LIMITS.padding) ?? fallback.padding,
    radius: clampNumber(value.radius, LIMITS.radius) ?? fallback.radius,
    shadow: clampNumber(value.shadow, LIMITS.shadow) ?? fallback.shadow,
    ratio: RATIOS.includes(value.ratio as Ratio) ? (value.ratio as Ratio) : fallback.ratio,
    watermark: typeof value.watermark === "string" ? value.watermark.slice(0, WATERMARK_MAX_LENGTH) : fallback.watermark,
  };
}

function parseBackground(value: unknown): StoredBackground | null {
  if (!isRecord(value)) return null;
  if (value.kind === "solid" && isHexColor(value.color)) return { kind: "solid", color: value.color };
  if (value.kind === "wallpaper") return { kind: "wallpaper" };
  if (value.kind === "gradient" && Number.isInteger(value.index)) {
    const index = value.index as number;
    if (index >= 0 && index < GRADIENT_PRESETS.length) return { kind: "gradient", index };
  }
  return null;
}

function clampNumber(value: unknown, max: number): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.min(Math.max(Math.round(value), 0), max);
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
