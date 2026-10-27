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
  /** Settles when every write started so far has finished. */
  private writing: Promise<void> = Promise.resolve();

  constructor(private readonly onError: (id: string, error: unknown) => void) {}

  schedule(id: string, build: () => CaptureDocument): void {
    if (this.pending && this.pending.id !== id) this.flush();
    this.pending = { id, build };
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.flush(), SAVE_DELAY_MS);
  }

  /**
   * Builds the pending document synchronously and resolves once it is stored (or failed,
   * which is reported through `onError`), so readers of the database can wait for it.
   */
  flush(): Promise<void> {
    window.clearTimeout(this.timer);
    const pending = this.pending;
    this.pending = null;
    if (!pending) return this.writing;
    const document = pending.build();
    const json = JSON.stringify(document);
    if (this.saved.get(pending.id) === json) return this.writing;
    this.saved.set(pending.id, json);
    const write = invoke("save_document", { id: pending.id, document }).catch((error) => {
      this.saved.delete(pending.id);
      this.onError(pending.id, error);
    });
    this.writing = Promise.all([this.writing, write]).then(() => undefined);
    return this.writing;
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
