import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import appIcon from "../src-tauri/icons/128x128@2x.png";
import { ShortcutSettings } from "./shortcut-settings";

/** As Rust reports them in one call (see `app_settings.rs`). */
type AppSettings = {
  soundOn: boolean;
  closeAfterCopy: boolean;
  hideWindowsOnCapture: boolean;
  hideDockIcon: boolean;
  defaultFileName: string;
  saveFolder: string | null;
  historyFolder: string | null;
  version: string;
};
type Toggle = "soundOn" | "closeAfterCopy" | "hideWindowsOnCapture";

/** Heard by the editor, which names the zone shortcut while it is empty. */
const SHORTCUTS_CHANGED = "shortcuts-changed";
const NO_SAVE_FOLDER = "Sin elegir: se preguntará al guardar con ⌘S";

const status = document.querySelector<HTMLElement>("#settings-status")!;
const saveFolder = document.querySelector<HTMLElement>("#save-folder")!;
const fileName = document.querySelector<HTMLInputElement>("#file-name")!;
const dockHidden = document.querySelector<HTMLInputElement>("#dock-hidden")!;

function showStatus(message: string, isError = false): void {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

/** Runs a Rust command, showing its error under the settings instead of failing silently. */
async function run<T>(command: string, args: Record<string, unknown>, failure: string): Promise<T | undefined> {
  try {
    const result = await invoke<T>(command, args);
    showStatus("");
    return result;
  } catch (error) {
    showStatus(`${failure}: ${error}`, true);
    return undefined;
  }
}

/**
 * Long paths are cut at the start so the folder name stays visible; the marks keep the
 * right-to-left trimming from moving the slashes.
 */
function showPath(element: HTMLElement, path: string | null, empty: string): void {
  element.textContent = path ? `\u200e${path}\u200e` : empty;
  element.title = path ?? "";
  element.classList.toggle("empty", path === null);
}

function showSaveFolder(path: string | null): void {
  showPath(saveFolder, path, NO_SAVE_FOLDER);
  document.querySelector<HTMLButtonElement>("#save-folder-open")!.disabled = path === null;
}

async function load(): Promise<void> {
  const settings = await invoke<AppSettings>("get_app_settings");
  document.querySelector<HTMLElement>("#app-version")!.textContent = settings.version;
  document.querySelectorAll<HTMLInputElement>("[data-toggle]").forEach((checkbox) => {
    checkbox.checked = settings[checkbox.dataset.toggle as Toggle];
  });
  dockHidden.checked = settings.hideDockIcon;
  fileName.value = settings.defaultFileName;
  showSaveFolder(settings.saveFolder);
  showPath(document.querySelector<HTMLElement>("#history-folder")!, settings.historyFolder, "No disponible");
}

function bind(): void {
  document.querySelector<HTMLButtonElement>("#capture-now")!.addEventListener("click", () => {
    void run("start_capture", {}, "No se pudo capturar");
  });

  document.querySelectorAll<HTMLInputElement>("[data-toggle]").forEach((checkbox) => {
    checkbox.addEventListener("change", async () => {
      const toggle = checkbox.dataset.toggle as Toggle;
      const saved = await run("set_app_toggle", { toggle, value: checkbox.checked }, "No se pudo guardar el ajuste");
      if (saved === undefined) checkbox.checked = !checkbox.checked;
    });
  });

  dockHidden.addEventListener("change", async () => {
    const hidden = dockHidden.checked;
    try {
      await invoke("set_dock_hidden", { hidden });
      showStatus("");
    } catch (error) {
      dockHidden.checked = !hidden;
      showStatus(`No se pudo cambiar el icono del Dock: ${error}`, true);
    }
  });

  document.querySelector<HTMLButtonElement>("#save-folder-open")!.addEventListener("click", () => {
    void run("open_save_folder", {}, "No se pudo abrir la carpeta");
  });
  document.querySelector<HTMLButtonElement>("#save-folder-change")!.addEventListener("click", async () => {
    const path = await run<string | null>("choose_save_folder", {}, "No se pudo cambiar la carpeta");
    if (path) showSaveFolder(path);
  });

  document.querySelector<HTMLFormElement>("#file-name-form")!.addEventListener("submit", async (event) => {
    event.preventDefault();
    const stored = await run<string>("set_file_name_pattern", { pattern: fileName.value }, "No se pudo guardar el nombre");
    if (stored !== undefined) {
      fileName.value = stored;
      showStatus("Nombre de archivo guardado");
    }
  });

  document.querySelector<HTMLButtonElement>("#history-open")!.addEventListener("click", () => {
    void run("open_history_folder", {}, "No se pudo abrir la carpeta del historial");
  });
  document.querySelector<HTMLButtonElement>("#history-show")!.addEventListener("click", () => {
    void run("show_history", {}, "No se pudo abrir el historial");
  });
}

document.querySelector<HTMLImageElement>("#app-icon")!.src = appIcon;
if (isTauri()) {
  const shortcuts = new ShortcutSettings(document.querySelector<HTMLUListElement>("#shortcut-list")!, (entries) => {
    emit(SHORTCUTS_CHANGED, entries).catch((error) => console.error("shortcuts-changed emit failed", error));
  });
  bind();
  // Leaving the window ends a recording, so the global shortcuts never stay paused.
  window.addEventListener("blur", () => shortcuts.stop());
  Promise.all([load(), shortcuts.load()]).catch((error) => showStatus(`No se pudieron leer los ajustes: ${error}`, true));
} else {
  showStatus("Esta ventana solo funciona dentro de Snapclone.", true);
}
