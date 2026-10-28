import type { ShortcutEntry } from "./shortcut-settings";

const NO_SHORTCUT = "La captura de zona no tiene atajo: usa el botón o el icono de la barra de menús.";

function kbd(text: string): HTMLElement {
  const key = document.createElement("kbd");
  key.textContent = text;
  return key;
}

/** The empty editor names the two zone shortcuts as they are configured right now. */
export function showZoneShortcuts(hint: HTMLElement, entries: ShortcutEntry[]): void {
  const label = (action: ShortcutEntry["action"]) => entries.find((entry) => entry.action === action)?.label ?? null;
  const open = label("region");
  const copy = label("region-copy");
  if (open && copy) {
    hint.replaceChildren(
      "Pulsa ",
      kbd(open),
      " para capturar y editar, o ",
      kbd(copy),
      " para capturar y copiar. También puedes usar el icono de la barra de menús.",
    );
  } else if (open) {
    hint.replaceChildren("Pulsa ", kbd(open), " o usa el icono de la barra de menús.");
  } else if (copy) {
    hint.replaceChildren("Pulsa ", kbd(copy), " para capturar y copiar, o usa el icono de la barra de menús.");
  } else {
    hint.textContent = NO_SHORTCUT;
  }
}
