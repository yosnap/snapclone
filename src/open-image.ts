import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Notify = (message: string, isError?: boolean) => void;

/**
 * Opening images that were not captured here. Rust reads the pasteboard and does the
 * converting; a result arrives as a normal `capture-ready`, a problem as `editor-notice`.
 */
export function openClipboard(notify: Notify): void {
  invoke("open_clipboard_image").catch((error) => notify(`No se pudo abrir el portapapeles: ${error}`, true));
}

/** Messages from actions started outside the editor, such as the menu bar icon. */
export function listenNotices(notify: Notify): void {
  listen<{ message: string; error: boolean }>("editor-notice", (event) => {
    notify(event.payload.message, event.payload.error);
  }).catch((error) => console.error("editor notices unavailable", error));
}
