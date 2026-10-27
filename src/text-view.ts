import { invoke } from "@tauri-apps/api/core";

/** One recognized line; the box is normalized (0..1) with the origin at the top-left. */
type TextLine = { text: string; x: number; y: number; width: number; height: number };

/**
 * In-window overlay with the text Vision recognized in the open capture, in reading order.
 * Rust caches the analysis per capture, so opening it after automatic redaction is instant.
 */
export class TextView {
  private readonly root = document.querySelector<HTMLElement>("#text-view")!;
  private readonly list = document.querySelector<HTMLOListElement>("#text-lines")!;
  private readonly empty = document.querySelector<HTMLElement>("#text-empty")!;
  private readonly copyButton = document.querySelector<HTMLButtonElement>("#text-copy")!;
  private readonly status = document.querySelector<HTMLElement>("#text-status")!;
  /** The capture being read; a newer request or closing makes older answers stale. */
  private showing: string | null = null;
  private returnFocus: HTMLElement | null = null;

  constructor() {
    document.querySelector<HTMLButtonElement>("#text-close")!.addEventListener("click", () => this.close());
    this.copyButton.addEventListener("click", () => this.copyAll());
    this.root.addEventListener("click", (event) => {
      if (event.target === this.root) this.close();
    });
    this.root.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        this.close();
      }
      // Editor shortcuts must not act on the capture behind; ⌘C still copies a selection.
      event.stopPropagation();
    });
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  async show(captureId: string): Promise<void> {
    this.showing = captureId;
    this.list.replaceChildren();
    this.empty.hidden = true;
    this.copyButton.disabled = true;
    this.showStatus("Leyendo el texto…");
    if (this.root.hidden) {
      this.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      this.root.hidden = false;
    }
    document.querySelector<HTMLButtonElement>("#text-close")!.focus();

    let lines: TextLine[];
    try {
      lines = await invoke<TextLine[]>("recognize_text", { id: captureId });
    } catch (error) {
      if (this.showing === captureId) this.showStatus(`No se pudo leer el texto: ${error}`, true);
      return;
    }
    if (this.showing !== captureId) return;
    this.list.replaceChildren(
      ...lines.map((line) => {
        const item = document.createElement("li");
        item.textContent = line.text;
        return item;
      }),
    );
    this.empty.hidden = lines.length > 0;
    this.copyButton.disabled = lines.length === 0;
    this.showStatus(lines.length > 0 ? `${lines.length} líneas` : "");
  }

  close(): void {
    if (this.root.hidden) return;
    this.showing = null;
    this.root.hidden = true;
    this.list.replaceChildren();
    this.returnFocus?.focus();
  }

  /** Rust joins the cached lines itself, the same way the menu bar's copy does. */
  private async copyAll(): Promise<void> {
    if (!this.showing) return;
    try {
      await invoke<number>("copy_capture_text", { id: this.showing });
      this.showStatus("Texto copiado al portapapeles");
    } catch (error) {
      this.showStatus(`No se pudo copiar el texto: ${error}`, true);
    }
  }

  private showStatus(message: string, isError = false): void {
    this.status.textContent = message;
    this.status.classList.toggle("error", isError);
  }
}
