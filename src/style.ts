export type GradientStop = { offset: number; color: string };

export type Background =
  | { kind: "gradient"; angle: number; stops: GradientStop[] }
  | { kind: "solid"; color: string }
  | { kind: "image"; image: HTMLImageElement }
  /** The desktop picture, blurred; `image` is filled in once it has loaded (see wallpaper.ts). */
  | { kind: "wallpaper"; image: HTMLImageElement | null };

/** All lengths are in points; the renderer multiplies them by the capture scale. */
export type Style = {
  background: Background;
  padding: number;
  radius: number;
  shadow: number;
  ratio: Ratio;
  watermark: string;
};

export type Ratio = "auto" | "1:1" | "4:3" | "3:2" | "16:9" | "9:16";

export const RATIOS: Ratio[] = ["auto", "1:1", "4:3", "3:2", "16:9", "9:16"];

/** Gradients taken from the presets embedded in Xnapper (CSS angles, 0deg = upwards). */
export const GRADIENT_PRESETS: Extract<Background, { kind: "gradient" }>[] = [
  gradient(50, ["hsla(297, 59%, 34%, 1)", "hsla(345, 74%, 58%, 1)", "hsla(22, 87%, 56%, 1)"]),
  gradient(50, ["#02aab0", "#00cdac"]),
  gradient(50, ["hsla(331, 78%, 69%, 1)", "hsla(238, 82%, 70%, 1)"]),
  gradient(50, ["hsla(31, 90%, 76%, 1)", "hsla(302, 82%, 76%, 1)"]),
  gradient(50, ["hsla(29, 92%, 70%, 1)", "hsla(0, 87%, 73%, 1)"]),
  gradient(160, ["#0093E9", "#80D0C7"]),
  gradient(90, ["#ED8F03", "#FFB75E"]),
  gradient(90, ["#F15F79", "#B24592"]),
  gradient(90, ["#8E54E9", "#4776E6"]),
];

/** Served by Rust from the main display's desktop picture; the page never names a file. */
export const WALLPAPER_URL = "capture://localhost/wallpaper";

/** The single wallpaper background, so it is recognised by identity like the gradient presets. */
export const WALLPAPER_BACKGROUND: Extract<Background, { kind: "wallpaper" }> = { kind: "wallpaper", image: null };

function gradient(angle: number, colors: string[]): Extract<Background, { kind: "gradient" }> {
  const last = Math.max(1, colors.length - 1);
  return {
    kind: "gradient",
    angle,
    stops: colors.map((color, index) => ({ offset: index / last, color })),
  };
}

export function defaultStyle(): Style {
  return {
    background: GRADIENT_PRESETS[0],
    padding: 64,
    radius: 12,
    shadow: 40,
    ratio: "auto",
    watermark: "",
  };
}

export function ratioValue(ratio: Ratio): number | null {
  if (ratio === "auto") return null;
  const [width, height] = ratio.split(":").map(Number);
  return width / height;
}

export function backgroundPreviewCss(background: Background): string {
  switch (background.kind) {
    case "gradient": {
      const stops = background.stops.map((stop) => `${stop.color} ${stop.offset * 100}%`);
      return `linear-gradient(${background.angle}deg, ${stops.join(", ")})`;
    }
    case "solid":
      return background.color;
    case "image":
      return `center / cover url("${background.image.src}")`;
    case "wallpaper":
      return `center / cover url("${WALLPAPER_URL}")`;
  }
}
