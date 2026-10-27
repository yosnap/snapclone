import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Notify = (message: string, isError?: boolean) => void;

/**
 * Tells Rust whether a capture is open. While one is, ⇧⌘Z stays redo; without one (or with
 * the editor hidden) the app menu takes it for "Reabrir última captura".
 */
export function reportLoaded(loaded: boolean): void {
  invoke("set_editor_loaded", { loaded }).catch((error) => console.error("editor state could not be reported", error));
}

/**
 * Reopens what Rust picks as the newest capture. `open` loads it with its document; ⇧⌘Z
 * typed in a text field of the empty editor still redoes the typing there.
 */
export function bindReopen(open: (id: string) => Promise<void>, notify: Notify): void {
  const failed = (error: unknown) => notify(`No se pudo reabrir la captura: ${error}`, true);
  listen<string>("reopen-capture", (event) => open(event.payload).catch(failed)).catch(failed);
  listen("reopen-shortcut", () => {
    const field = document.activeElement;
    if (field instanceof HTMLInputElement && field.type === "text") document.execCommand("redo");
    else invoke("reopen_last_capture").catch(failed);
  }).catch(failed);
}
