import type { StoredStyle } from "./persisted-style";
import { type MenuItem, closePopover, openPopover, showMenu } from "./popover";
import { PRESETS_MAX, PRESET_NAME_MAX_LENGTH, type Preset, normalizePresetName } from "./settings";

type PresetHandlers = {
  /** Snapshot of the style currently shown, to store under a name. */
  current: () => StoredStyle;
  apply: (style: StoredStyle) => void;
  persist: (presets: Preset[]) => void;
  notify: (message: string, isError?: boolean) => void;
};

const UNNAMED = "Tu preset";

/**
 * The preset pop-up: picking a preset applies it, "Guardar preset actual…" stores the style
 * shown under a name (replacing a preset with that name) and the trash button deletes the
 * preset shown.
 */
export function bindPresets(container: HTMLElement, initial: Preset[], handlers: PresetHandlers): void {
  let presets = [...initial];
  let selected: string | null = null;
  const button = container.querySelector<HTMLButtonElement>("#preset-button")!;
  const label = container.querySelector<HTMLElement>("#preset-label")!;
  const deleteButton = container.querySelector<HTMLButtonElement>("#preset-delete")!;

  const sync = () => {
    label.textContent = selected ?? UNNAMED;
    deleteButton.disabled = selected === null;
    deleteButton.title = selected ? `Borrar el preset «${selected}»` : "Borrar preset";
  };

  const save = (name: string) => {
    const presetName = normalizePresetName(name);
    if (!presetName) return false;
    const replaced = presets.some((preset) => preset.name === presetName);
    if (!replaced && presets.length >= PRESETS_MAX) {
      handlers.notify(`Solo caben ${PRESETS_MAX} presets: borra alguno antes`, true);
      return false;
    }
    const preset = { name: presetName, style: handlers.current() };
    presets = replaced ? presets.map((item) => (item.name === presetName ? preset : item)) : [...presets, preset];
    selected = presetName;
    handlers.persist(presets);
    sync();
    handlers.notify(replaced ? `Preset «${presetName}» actualizado` : `Preset «${presetName}» guardado`);
    return true;
  };

  button.addEventListener("click", () => {
    const items: MenuItem[] = presets.map((preset) => ({
      label: preset.name,
      checked: preset.name === selected,
      action: () => {
        selected = preset.name;
        sync();
        handlers.apply(preset.style);
      },
    }));
    if (items.length === 0) items.push({ label: "Sin presets guardados", disabled: true, action: () => {} });
    items.push("separator", { label: "Guardar preset actual…", action: () => openNameEntry(button, selected ?? "", save) });
    showMenu(button, items);
  });

  deleteButton.addEventListener("click", () => {
    if (!selected) return;
    const presetName = selected;
    presets = presets.filter((preset) => preset.name !== presetName);
    selected = null;
    handlers.persist(presets);
    sync();
    handlers.notify(`Preset «${presetName}» borrado`);
  });

  sync();
}

/** "Escribe el nombre del preset" with Guardar (↩). */
function openNameEntry(anchor: HTMLElement, suggested: string, save: (name: string) => boolean): void {
  const form = document.createElement("form");
  form.className = "name-entry";
  const hint = document.createElement("p");
  hint.className = "popover-caption";
  hint.textContent = "Escribe el nombre del preset";
  const input = document.createElement("input");
  input.type = "text";
  input.className = "text-field";
  input.placeholder = "Nombre del preset";
  input.maxLength = PRESET_NAME_MAX_LENGTH;
  input.value = suggested;
  const done = document.createElement("button");
  done.type = "submit";
  done.className = "push-button default";
  done.textContent = "Guardar";
  const line = document.createElement("div");
  line.className = "popover-row";
  line.append(input, done);
  form.append(hint, line);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    if (save(input.value)) closePopover();
  });
  openPopover(anchor, form);
  requestAnimationFrame(() => input.select());
}
