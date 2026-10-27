import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Notify } from "./ui-helpers";

/** ⇧⌘Z in the empty editor: Rust picks the newest capture and answers with `reopen-capture`. */
export function reopenLast(notify: Notify): void {
  invoke("reopen_last_capture").catch((error) => notify(`No se pudo reabrir la captura: ${error}`, true));
}

/** Opens what Rust picked as the newest capture; `open` loads it with its document. */
export function bindReopen(open: (id: string) => Promise<void>, notify: Notify): void {
  const failed = (error: unknown) => notify(`No se pudo reabrir la captura: ${error}`, true);
  listen<string>("reopen-capture", (event) => open(event.payload).catch(failed)).catch(failed);
}
