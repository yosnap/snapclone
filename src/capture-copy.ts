import { invoke } from "@tauri-apps/api/core";

/** What the editor page lends "Capturar zona y copiar", which never shows the editor. */
export type CopyHooks = {
  /** Opens the capture as a new one, with the current style, like any other capture. */
  load: () => Promise<void>;
  /** Settles once automatic redaction has been applied (or skipped); false if it failed. */
  redacted: () => Promise<boolean>;
  /** Renders and copies like the Copiar button; `failed` hears why it did not. */
  copy: (failed: (message: string) => void) => Promise<boolean>;
};

const COPY_FAILED = "No se pudo copiar la captura";
const REDACTION_FAILED =
  "No se ha copiado: no se pudieron buscar datos sensibles para tacharlos. Revisa la captura antes de copiarla";

/**
 * Styles and copies a capture while the editor stays hidden, then tells Rust, which shows
 * the "copied" bezel or brings the editor forward with the reason. The capture is in the
 * history already, and stays open in the editor for whoever opens it next. A capture whose
 * text could not be analysed is never copied unreviewed: automatic redaction is what keeps
 * sensitive data out of what the user pastes without looking.
 */
export async function copyCapture(id: string, hooks: CopyHooks): Promise<void> {
  let error: string | null = null;
  try {
    await hooks.load();
    if (!(await hooks.redacted())) {
      error = REDACTION_FAILED;
    } else {
      let failure = COPY_FAILED;
      const copied = await hooks.copy((message) => (failure = message));
      if (!copied) error = failure;
    }
  } catch (problem) {
    error = `${COPY_FAILED}: ${problem}`;
  }
  try {
    await invoke("copy_finished", { id, error });
  } catch (problem) {
    // Rust shows the editor on its own if no answer arrives in time.
    console.error("copy_finished failed", problem);
  }
}
