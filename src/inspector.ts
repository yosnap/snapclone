import { bindBackgroundPicker } from "./background-picker";
import { closePopover, isOpenFor, openPopover } from "./popover";
import {
  type Background,
  LIMITS,
  type NumericKey,
  RATIO_CHOICES,
  type RatioChoice,
  SOCIAL_SIZES,
  type Style,
  parseRatio,
  ratioText,
} from "./style";

type InspectorHandlers = {
  /** After every edit the user makes; `style` has already been changed. */
  changed: () => void;
  /** Redraws without saving, while a colour picker is being dragged. */
  previewed: () => void;
  wallpaperPicked: () => void;
  /** The detected edge colour of the capture shown, or null without one. */
  edgeColor: () => string | null;
};

const NUMERIC_KEYS: NumericKey[] = ["padding", "inset", "radius", "shadow"];

/**
 * Wires the panel to `style`, mutating it in place and calling `changed` after every edit.
 * Returns a function that redraws the controls from `style`, for changes made elsewhere
 * (a preset, a restored capture, a new capture's edge colour).
 */
export function bindInspector(root: HTMLElement, style: Style, handlers: InspectorHandlers): () => void {
  const sliders = NUMERIC_KEYS.map((key) => bindSlider(root, style, key, handlers.changed));

  const refreshBackgrounds = bindBackgroundPicker(root.querySelector<HTMLElement>("#background-grid")!, {
    current: () => style.background,
    select: (background: Background) => {
      style.background = background;
      refreshBackgrounds();
      handlers.changed();
    },
    preview: (background: Background) => {
      style.background = background;
      refreshBackgrounds();
      handlers.previewed();
    },
    wallpaperPicked: handlers.wallpaperPicked,
  });

  const refreshRatios = bindRatios(root, style, handlers.changed);

  const insetColor = root.querySelector<HTMLInputElement>("#inset-color")!;
  const insetStatus = root.querySelector<HTMLElement>("#inset-status")!;
  insetColor.addEventListener("input", () => {
    style.insetColor = insetColor.value;
    refreshInset();
    handlers.previewed();
  });
  insetColor.addEventListener("change", () => {
    style.insetColor = insetColor.value;
    handlers.changed();
  });
  const refreshInset = () => {
    const detected = handlers.edgeColor();
    const shown = style.insetColor ?? detected;
    insetColor.value = shown ? toHex(shown) : "#ffffff";
    insetColor.disabled = detected === null;
    insetStatus.textContent = style.insetColor ? "(color elegido)" : detected ? "(color de fondo detectado)" : "";
  };

  const showWatermark = root.querySelector<HTMLInputElement>("#watermark-toggle")!;
  const watermark = root.querySelector<HTMLInputElement>('input[name="watermark"]')!;
  showWatermark.addEventListener("change", () => {
    style.showWatermark = showWatermark.checked;
    watermark.disabled = !showWatermark.checked;
    handlers.changed();
  });
  watermark.addEventListener("input", () => {
    style.watermark = watermark.value;
    handlers.changed();
  });

  const refresh = () => {
    sliders.forEach((refreshSlider) => refreshSlider());
    refreshBackgrounds();
    refreshRatios();
    refreshInset();
    showWatermark.checked = style.showWatermark;
    watermark.value = style.watermark;
    watermark.disabled = !style.showWatermark;
  };
  refresh();
  return refresh;
}

/** A slider with its small grey value; clicking the value lets the user type it. */
function bindSlider(root: HTMLElement, style: Style, key: NumericKey, changed: () => void): () => void {
  const input = root.querySelector<HTMLInputElement>(`input[name="${key}"]`)!;
  const value = root.querySelector<HTMLButtonElement>(`[data-value-for="${key}"]`)!;
  input.min = "0";
  input.max = String(LIMITS[key]);
  const show = () => {
    input.value = String(style[key]);
    value.textContent = input.value;
    input.style.setProperty("--fill", `${(style[key] / LIMITS[key]) * 100}%`);
  };
  const set = (next: number) => {
    style[key] = Math.min(LIMITS[key], Math.max(0, Math.round(next)));
    show();
    changed();
  };
  input.addEventListener("input", () => set(Number(input.value)));
  value.addEventListener("click", () => {
    if (isOpenFor(value)) {
      closePopover();
      return;
    }
    openPopover(value, numberEntry(style[key], LIMITS[key], set));
  });
  show();
  return show;
}

/** "Escribe un número" with Aplicar (↩), like Xnapper's value popover. */
function numberEntry(current: number, max: number, apply: (value: number) => void): HTMLElement {
  const form = document.createElement("form");
  form.className = "number-entry";
  const input = document.createElement("input");
  input.type = "number";
  input.className = "text-field";
  input.min = "0";
  input.max = String(max);
  input.value = String(current);
  const button = document.createElement("button");
  button.type = "submit";
  button.className = "push-button default";
  button.textContent = "Aplicar";
  const hint = document.createElement("p");
  hint.className = "popover-caption";
  hint.textContent = `Escribe un número entre 0 y ${max}`;
  form.append(input, button, hint);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const number = Number(input.value);
    if (input.value.trim() === "" || !Number.isFinite(number)) return;
    apply(number);
    closePopover();
  });
  requestAnimationFrame(() => input.select());
  return form;
}

/** "Ratio / Size": one text button per choice, the selected one outlined, and "Personalizado…". */
function bindRatios(root: HTMLElement, style: Style, changed: () => void): () => void {
  const grid = root.querySelector<HTMLElement>("#ratio-grid")!;
  const custom = root.querySelector<HTMLButtonElement>("#ratio-custom")!;
  const buttons: { button: HTMLButtonElement; choice: RatioChoice }[] = [];
  const select = (value: string) => {
    style.ratio = value;
    refresh();
    changed();
  };
  const row = (choices: RatioChoice[], className: string) => {
    const line = document.createElement("div");
    line.className = `ratio-row ${className}`;
    for (const choice of choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "ratio-button";
      button.textContent = choice.label;
      if (choice.value.includes("x")) button.title = `${choice.label}: ${choice.value.replace("x", " × ")} px`;
      button.addEventListener("click", () => select(choice.value));
      buttons.push({ button, choice });
      line.append(button);
    }
    grid.append(line);
  };
  row(RATIO_CHOICES, "five");
  row(SOCIAL_SIZES.slice(0, 4), "four");
  row(SOCIAL_SIZES.slice(4), "four");

  custom.addEventListener("click", () => {
    if (isOpenFor(custom)) {
      closePopover();
      return;
    }
    openPopover(custom, ratioEntry(style.ratio, select), "below");
  });

  const refresh = () => {
    let matched = false;
    for (const { button, choice } of buttons) {
      const selected = choice.value === style.ratio;
      matched ||= selected;
      button.setAttribute("aria-pressed", String(selected));
    }
    custom.setAttribute("aria-pressed", String(!matched));
    custom.textContent = matched ? "Personalizado…" : `${style.ratio.replace("x", " × ")}…`;
  };
  return refresh;
}

function ratioEntry(current: string, apply: (value: string) => void): HTMLElement {
  const form = document.createElement("form");
  form.className = "ratio-entry";
  const hint = document.createElement("p");
  hint.className = "popover-caption";
  hint.textContent = "Escribe una proporción (p. ej. 1:2, 4:3) o un tamaño (p. ej. 400x300, 1920x1080)";
  const input = document.createElement("input");
  input.type = "text";
  input.className = "text-field";
  input.value = current === "auto" ? "" : current;
  input.placeholder = "16:9 o 1920x1080";
  const button = document.createElement("button");
  button.type = "submit";
  button.className = "push-button default";
  button.textContent = "Aplicar";
  const error = document.createElement("p");
  error.className = "popover-caption error";
  error.hidden = true;
  error.textContent = "No es una proporción ni un tamaño válido (de 16 a 8000 px por lado)";
  const line = document.createElement("div");
  line.className = "popover-row";
  line.append(input, button);
  form.append(hint, line, error);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const spec = parseRatio(input.value);
    error.hidden = spec !== null;
    if (!spec) return;
    apply(ratioText(spec));
    closePopover();
  });
  requestAnimationFrame(() => input.focus());
  return form;
}

/** `<input type=color>` only takes #rrggbb; edge colours come as rgba(). */
function toHex(color: string): string {
  if (color.startsWith("#")) return color.slice(0, 7);
  const [red, green, blue] = (color.match(/\d+(\.\d+)?/g) ?? ["255", "255", "255"]).map(Number);
  return `#${[red, green, blue].map((channel) => Math.round(channel).toString(16).padStart(2, "0")).join("")}`;
}
