import { REDACT_CATEGORIES, type RedactCategory, type RedactMode, type RedactSettings } from "./auto-redact";
import { closePopover, isOpenFor, openPopover } from "./popover";

export type RedactPreferences = { enabled: boolean; mode: RedactMode; categories: RedactSettings };

type RedactHandlers = {
  /** The main checkbox changed. */
  enabledChanged: (enabled: boolean) => void;
  categoryChanged: (category: RedactCategory, enabled: boolean) => void;
  modeChanged: (mode: RedactMode) => void;
};

const MODE_LABELS: Record<RedactMode, string> = { pixelate: "Pixelado", solid: "Bloque sólido" };

/**
 * "Redactar datos sensibles": the main checkbox, how many regions were found, and the gear
 * popover with the categories and how they are hidden. Returns a function that shows the
 * search state: a number found, or null while searching.
 */
export function bindRedactPanel(root: HTMLElement, preferences: RedactPreferences, handlers: RedactHandlers) {
  const toggle = root.querySelector<HTMLInputElement>("#redact-toggle")!;
  const count = root.querySelector<HTMLElement>("#redact-count")!;
  const gear = root.querySelector<HTMLButtonElement>("#redact-settings")!;
  toggle.checked = preferences.enabled;
  toggle.addEventListener("change", () => {
    preferences.enabled = toggle.checked;
    handlers.enabledChanged(toggle.checked);
  });
  gear.addEventListener("click", () => {
    if (isOpenFor(gear)) {
      closePopover();
      return;
    }
    openPopover(gear, settingsPanel(preferences, handlers), "left");
  });
  return (found: number | null) => {
    count.textContent = found === null ? "(buscando…)" : `(encontrados ${found})`;
  };
}

function settingsPanel(preferences: RedactPreferences, handlers: RedactHandlers): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "redact-settings";
  const title = document.createElement("p");
  title.className = "popover-title";
  title.textContent = "Redactar automáticamente";
  panel.append(title);
  for (const { id, label } of REDACT_CATEGORIES) {
    const row = document.createElement("label");
    row.className = "checkbox";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = preferences.categories[id];
    checkbox.addEventListener("change", () => {
      preferences.categories[id] = checkbox.checked;
      handlers.categoryChanged(id, checkbox.checked);
    });
    row.append(checkbox, label);
    panel.append(row);
  }

  const modeTitle = document.createElement("p");
  modeTitle.className = "popover-title spaced";
  modeTitle.textContent = "Cómo se tapan";
  panel.append(modeTitle);
  for (const mode of Object.keys(MODE_LABELS) as RedactMode[]) {
    const row = document.createElement("label");
    row.className = "checkbox";
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "redact-mode";
    radio.checked = preferences.mode === mode;
    radio.addEventListener("change", () => {
      if (!radio.checked) return;
      preferences.mode = mode;
      handlers.modeChanged(mode);
    });
    row.append(radio, MODE_LABELS[mode]);
    panel.append(row);
  }

  const note = document.createElement("p");
  note.className = "popover-caption";
  note.textContent = "El texto se detecta en tu Mac con el framework Vision de Apple: la captura no sale del equipo.";
  panel.append(note);
  return panel;
}
