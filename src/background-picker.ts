import { closePopover, isOpenFor, openPopover } from "./popover";
import {
  type Background,
  EXTRA_GRADIENTS,
  NAMED_BACKGROUNDS,
  WALLPAPER_BACKGROUND,
  backgroundPreviewCss,
} from "./style";

/** Shown on "Más…" until a background from its popover is picked. */
const MORE_PREVIEW = "linear-gradient(135deg, #ff9a9e 0 20%, #fad0c4 20% 40%, #fbc2eb 40% 60%, #a1c4fd 60% 80%, #c2e9fb 80%)";
const DEFAULT_SOLID = "#1e293b";

type PickerHandlers = {
  current: () => Background;
  select: (background: Background) => void;
  /** Shows a background without saving it, while the colour picker is dragged. */
  preview: (background: Background) => void;
  /** Runs every time the desktop picture is picked, so it is loaded or refreshed. */
  wallpaperPicked: () => void;
};

/**
 * "Fondo": the 2×5 grid of thumbnails with their names, and "Más…" with a plain colour, an
 * image of the user's and the remaining gradients. Returns a function that marks the
 * selected thumbnail.
 */
export function bindBackgroundPicker(grid: HTMLElement, handlers: PickerHandlers): () => void {
  const tiles: { button: HTMLButtonElement; background: Background }[] = [];
  for (const { label, background } of NAMED_BACKGROUNDS) {
    const button = tile(label, backgroundPreviewCss(background));
    button.addEventListener("click", () => {
      handlers.select(background);
      if (background === WALLPAPER_BACKGROUND) handlers.wallpaperPicked();
    });
    tiles.push({ button, background });
    grid.append(button);
  }
  const more = tile("Más…", MORE_PREVIEW);
  more.setAttribute("aria-haspopup", "dialog");
  grid.append(more);
  const moreSwatch = more.querySelector<HTMLElement>(".background-thumb")!;

  const refresh = () => {
    const current = handlers.current();
    let named = false;
    for (const { button, background } of tiles) {
      const selected = background === current;
      named ||= selected;
      button.setAttribute("aria-pressed", String(selected));
    }
    more.setAttribute("aria-pressed", String(!named));
    // The tile previews the custom background while it is the one in use.
    moreSwatch.style.background = named ? MORE_PREVIEW : backgroundPreviewCss(current);
  };

  more.addEventListener("click", () => {
    if (isOpenFor(more)) {
      closePopover();
      return;
    }
    openPopover(more, morePanel(handlers), "left");
  });
  return refresh;
}

function tile(label: string, preview: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "background-tile";
  button.title = label;
  const thumb = document.createElement("span");
  thumb.className = "background-thumb";
  thumb.style.background = preview;
  // Shown while the desktop picture loads, or if macOS gives none.
  if (preview.includes("url(")) thumb.style.backgroundColor = "var(--checker)";
  const name = document.createElement("span");
  name.className = "background-name";
  name.textContent = label;
  button.append(thumb, name);
  return button;
}

/** Only the latest picked image is kept alive; older blob URLs are released. */
let imageUrl: string | null = null;

function morePanel(handlers: PickerHandlers): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "more-backgrounds";
  const current = handlers.current();

  const solidRow = document.createElement("label");
  solidRow.className = "popover-row";
  const solid = document.createElement("input");
  solid.type = "color";
  solid.className = "color-well";
  solid.value = current.kind === "solid" ? current.color : DEFAULT_SOLID;
  // `click` applies the shown colour too; `input` only previews and `change` saves.
  const selectSolid = () => handlers.select({ kind: "solid", color: solid.value });
  solid.addEventListener("click", selectSolid);
  solid.addEventListener("input", () => handlers.preview({ kind: "solid", color: solid.value }));
  solid.addEventListener("change", selectSolid);
  solidRow.append(solid, "Color liso");

  const imageRow = document.createElement("div");
  imageRow.className = "popover-row";
  const file = document.createElement("input");
  file.type = "file";
  file.accept = "image/*";
  file.hidden = true;
  const choose = document.createElement("button");
  choose.type = "button";
  choose.className = "push-button";
  choose.textContent = "Elegir…";
  choose.addEventListener("click", () => file.click());
  file.addEventListener("change", () => void pickImage(file, handlers));
  imageRow.append(choose, file, "Imagen propia");

  const title = document.createElement("p");
  title.className = "popover-caption";
  title.textContent = "Otros fondos";
  const gradients = document.createElement("div");
  gradients.className = "background-grid compact";
  EXTRA_GRADIENTS.forEach((gradient, index) => {
    const button = tile(`Degradado ${index + 1}`, backgroundPreviewCss(gradient));
    button.setAttribute("aria-pressed", String(gradient === current));
    button.addEventListener("click", () => {
      handlers.select(gradient);
      closePopover();
    });
    gradients.append(button);
  });

  panel.append(solidRow, imageRow, title, gradients);
  return panel;
}

async function pickImage(file: HTMLInputElement, handlers: PickerHandlers): Promise<void> {
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
  if (imageUrl) URL.revokeObjectURL(imageUrl);
  imageUrl = url;
  handlers.select({ kind: "image", image });
  closePopover();
}
