import type { StoredStyle } from "./persisted-style";
import { PRESETS_MAX, PRESET_NAME_MAX_LENGTH, type Preset, normalizePresetName } from "./settings";

type PresetHandlers = {
  /** Snapshot of the style currently shown, to store under a name. */
  current: () => StoredStyle;
  apply: (style: StoredStyle) => void;
  persist: (presets: Preset[]) => void;
  notify: (message: string) => void;
};

/**
 * Named styles: picking one applies it, "Guardar" stores the current style under the typed
 * name (replacing a preset with the same name) and "Borrar" removes the selected one.
 */
export function bindPresets(container: HTMLElement, initial: Preset[], handlers: PresetHandlers): void {
  let presets = [...initial];
  const select = container.querySelector<HTMLSelectElement>("#preset-select")!;
  const name = container.querySelector<HTMLInputElement>("#preset-name")!;
  const saveButton = container.querySelector<HTMLButtonElement>("#preset-save")!;
  const deleteButton = container.querySelector<HTMLButtonElement>("#preset-delete")!;
  name.maxLength = PRESET_NAME_MAX_LENGTH;

  const sync = (selected: string) => {
    const placeholder = new Option(presets.length > 0 ? "Elige un preset…" : "Sin presets guardados", "");
    select.replaceChildren(placeholder, ...presets.map((preset) => new Option(preset.name, preset.name)));
    select.value = presets.some((preset) => preset.name === selected) ? selected : "";
    syncButtons();
  };
  const syncButtons = () => {
    const typed = normalizePresetName(name.value);
    const full = presets.length >= PRESETS_MAX && !presets.some((preset) => preset.name === typed);
    saveButton.disabled = !typed || full;
    deleteButton.disabled = !select.value;
  };

  select.addEventListener("change", () => {
    const preset = presets.find((item) => item.name === select.value);
    if (preset) {
      name.value = preset.name;
      handlers.apply(preset.style);
    }
    syncButtons();
  });
  name.addEventListener("input", syncButtons);
  name.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.isComposing && !saveButton.disabled) saveButton.click();
  });

  saveButton.addEventListener("click", () => {
    const presetName = normalizePresetName(name.value);
    if (!presetName) return;
    const replaced = presets.some((preset) => preset.name === presetName);
    const preset = { name: presetName, style: handlers.current() };
    presets = replaced
      ? presets.map((item) => (item.name === presetName ? preset : item))
      : [...presets, preset];
    handlers.persist(presets);
    sync(presetName);
    handlers.notify(replaced ? `Preset «${presetName}» actualizado` : `Preset «${presetName}» guardado`);
  });

  deleteButton.addEventListener("click", () => {
    const presetName = select.value;
    if (!presetName) return;
    presets = presets.filter((preset) => preset.name !== presetName);
    handlers.persist(presets);
    name.value = "";
    sync("");
    handlers.notify(`Preset «${presetName}» borrado`);
  });

  sync("");
}
