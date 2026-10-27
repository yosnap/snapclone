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

/** Wires the sidebar to `style`, mutating it in place and calling `onChange` after every edit. */
export function bindControls(root: HTMLElement, style: Style, onChange: () => void): void {
  bindBackgrounds(root, style, onChange);

  for (const key of NUMERIC_KEYS) {
    const input = root.querySelector<HTMLInputElement>(`input[name="${key}"]`)!;
    const output = root.querySelector<HTMLOutputElement>(`output[data-for="${key}"]`)!;
    input.value = String(style[key]);
    output.value = input.value;
    input.addEventListener("input", () => {
      style[key] = Number(input.value);
      output.value = input.value;
      onChange();
    });
  }

  const ratio = root.querySelector<HTMLSelectElement>('select[name="ratio"]')!;
  ratio.replaceChildren(...RATIOS.map((value) => new Option(RATIO_LABELS[value], value)));
  ratio.value = style.ratio;
  ratio.addEventListener("change", () => {
    style.ratio = ratio.value as Ratio;
    onChange();
  });

  const watermark = root.querySelector<HTMLInputElement>('input[name="watermark"]')!;
  watermark.value = style.watermark;
  watermark.addEventListener("input", () => {
    style.watermark = watermark.value;
    onChange();
  });
}

function bindBackgrounds(root: HTMLElement, style: Style, onChange: () => void): void {
  const swatches = root.querySelector<HTMLElement>("#background-swatches")!;
  const solid = root.querySelector<HTMLInputElement>("#solid-color")!;
  const file = root.querySelector<HTMLInputElement>("#background-file")!;

  const select = (background: Background, swatch: HTMLElement | null) => {
    style.background = background;
    swatches.querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", "false"));
    swatch?.setAttribute("aria-pressed", "true");
    onChange();
  };

  GRADIENT_PRESETS.forEach((preset, index) => {
    const swatch = document.createElement("button");
    swatch.type = "button";
    swatch.className = "swatch";
    swatch.title = `Degradado ${index + 1}`;
    swatch.style.background = backgroundPreviewCss(preset);
    swatch.setAttribute("aria-pressed", String(style.background === preset));
    swatch.addEventListener("click", () => select(preset, swatch));
    swatches.append(swatch);
  });

  solid.addEventListener("input", () => select({ kind: "solid", color: solid.value }, null));

  file.addEventListener("change", async () => {
    const chosen = file.files?.[0];
    if (!chosen) return;
    const image = new Image();
    image.src = URL.createObjectURL(chosen);
    try {
      await image.decode();
    } catch (error) {
      console.error("background image could not be decoded", error);
      return;
    }
    if (style.background.kind === "image") URL.revokeObjectURL(style.background.image.src);
    select({ kind: "image", image }, null);
    file.value = "";
  });
}
