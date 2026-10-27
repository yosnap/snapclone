import { invoke } from "@tauri-apps/api/core";
import type { CaptureDocument } from "./capture-document";

const SAVE_DELAY_MS = 500;

/**
 * Writes a capture's editing state to the history shortly after the last change. The
 * document is built when the timer fires, from whatever the editor shows then, so callers
 * must `flush` before switching the editor to another capture.
 */
export class DocumentSaver {
  private pending: { id: string; build: () => CaptureDocument } | null = null;
  private timer: number | undefined;
  /** Last JSON written per capture, so selection changes and no-op edits cost no IPC. */
  private readonly saved = new Map<string, string>();

  constructor(private readonly onError: (id: string, error: unknown) => void) {}

  schedule(id: string, build: () => CaptureDocument): void {
    if (this.pending && this.pending.id !== id) this.flush();
    this.pending = { id, build };
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.flush(), SAVE_DELAY_MS);
  }

  /** Builds the pending document now; only the IPC call is asynchronous. */
  flush(): void {
    window.clearTimeout(this.timer);
    const pending = this.pending;
    this.pending = null;
    if (!pending) return;
    const document = pending.build();
    const json = JSON.stringify(document);
    if (this.saved.get(pending.id) === json) return;
    this.saved.set(pending.id, json);
    invoke("save_document", { id: pending.id, document }).catch((error) => {
      this.saved.delete(pending.id);
      this.onError(pending.id, error);
    });
  }

  /** Drops anything pending for a capture that no longer exists. */
  forget(id: string): void {
    if (this.pending?.id === id) {
      window.clearTimeout(this.timer);
      this.pending = null;
    }
    this.saved.delete(id);
  }
}
