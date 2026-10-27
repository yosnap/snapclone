import { type Background, type Ratio, type Style, GRADIENT_PRESETS, RATIOS, backgroundPreviewCss } from "./style";

const RATIO_LABELS: Record<Ratio, string> = {
  auto: "Automática",
  "1:1": "1:1 cuadrada",
  "4:3": "4:3",
  "3:2": "3:2",
  "16:9": "16:9 panorámica",
  "9:16": "9:16 vertical",
};

type NumericKey = "padding" | "radius" | "shadow";
const NUMERIC_KEYS: NumericKey[] = ["padding", "radius", "shadow"];

/**
 * Wires the sidebar to `style`, mutating it in place and calling `onChange` after every edit.
 * Returns a function that redraws the controls from `style`, for changes made elsewhere
 * (a preset, a restored capture).
 */
export function bindControls(root: HTMLElement, style: Style, onChange: () => void): () => void {
  const refreshBackgrounds = bindBackgrounds(root, style, onChange);

  const numeric = NUMERIC_KEYS.map((key) => {
    const input = root.querySelector<HTMLInputElement>(`input[name="${key}"]`)!;
    const output = root.querySelector<HTMLOutputElement>(`output[data-for="${key}"]`)!;
    input.addEventListener("input", () => {
      style[key] = Number(input.value);
      output.value = input.value;
      onChange();
    });
    return { key, input, output };
  });

  const ratio = root.querySelector<HTMLSelectElement>('select[name="ratio"]')!;
  ratio.replaceChildren(...RATIOS.map((value) => new Option(RATIO_LABELS[value], value)));
  ratio.addEventListener("change", () => {
    style.ratio = ratio.value as Ratio;
    onChange();
  });

  const watermark = root.querySelector<HTMLInputElement>('input[name="watermark"]')!;
  watermark.addEventListener("input", () => {
    style.watermark = watermark.value;
    onChange();
  });

  const refresh = () => {
    for (const { key, input, output } of numeric) {
      input.value = String(style[key]);
      output.value = input.value;
    }
    ratio.value = style.ratio;
    watermark.value = style.watermark;
    refreshBackgrounds();
  };
  refresh();
  return refresh;
}

function bindBackgrounds(root: HTMLElement, style: Style, onChange: () => void): () => void {
  const swatches = root.querySelector<HTMLElement>("#background-swatches")!;
  const solid = root.querySelector<HTMLInputElement>("#solid-color")!;
  const file = root.querySelector<HTMLInputElement>("#background-file")!;
  const buttons: HTMLButtonElement[] = [];

  const refresh = () => {
    buttons.forEach((button, index) => {
      button.setAttribute("aria-pressed", String(style.background === GRADIENT_PRESETS[index]));
    });
    if (style.background.kind === "solid") solid.value = style.background.color;
  };

  const select = (background: Background) => {
    style.background = background;
    refresh();
    onChange();
  };

  GRADIENT_PRESETS.forEach((preset, index) => {
    const swatch = document.createElement("button");
    swatch.type = "button";
    swatch.className = "swatch";
    swatch.title = `Degradado ${index + 1}`;
    swatch.style.background = backgroundPreviewCss(preset);
    swatch.addEventListener("click", () => select(preset));
    swatches.append(swatch);
    buttons.push(swatch);
  });

  // `click` applies the initial colour too; `input` alone only fires when it changes.
  const selectSolid = () => select({ kind: "solid", color: solid.value });
  solid.addEventListener("click", selectSolid);
  solid.addEventListener("input", selectSolid);

  // Only the latest uploaded background is kept alive; older blob URLs are released.
  let backgroundUrl: string | null = null;
  file.addEventListener("change", async () => {
    const chosen = file.files?.[0];
    file.value = "";
    if (!chosen) return;
    const url = URL.createObjectURL(chosen);
    const image = new Image();
    image.src = url;
    try {
      await image.decode();
    } catch (error) {
      URL.revokeObjectURL(url);
      console.error("background image could not be decoded", error);
      return;
    }
    if (backgroundUrl) URL.revokeObjectURL(backgroundUrl);
    backgroundUrl = url;
    select({ kind: "image", image });
  });
  return refresh;
}
