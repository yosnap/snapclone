import { invoke } from "@tauri-apps/api/core";

export type HistoryItem = { id: string; createdAt: number; scale: number };
type HistoryPage = { items: HistoryItem[]; hasMore: boolean };
export type OpenedCapture = { id: string; scale: number; document: unknown };

/** Captures are served by Rust under this scheme, with CORS so canvases stay exportable. */
export function captureUrl(id: string): string {
  return `capture://localhost/${encodeURIComponent(id)}`;
}

/** Small preview written next to each capture, so the grid never decodes full screenshots. */
function thumbnailUrl(id: string): string {
  return `capture://localhost/thumbnail/${encodeURIComponent(id)}`;
}

const DATE_FORMAT = new Intl.DateTimeFormat("es-ES", { dateStyle: "medium", timeStyle: "short" });

type HistoryHandlers = {
  /** Loads the capture into the editor; rejects with a message when it cannot. */
  open: (id: string) => Promise<void>;
  /** Lets the editor clear itself when the capture it shows is deleted. */
  deleted: (id: string) => void;
  notify: (message: string, isError?: boolean) => void;
};

/**
 * In-window overlay listing recent captures. Thumbnails are the stored PNGs served by
 * `capture://`; each entry reopens its capture or deletes it.
 */
export class HistoryView {
  private readonly root = document.querySelector<HTMLElement>("#history")!;
  private readonly list = document.querySelector<HTMLUListElement>("#history-list")!;
  private readonly empty = document.querySelector<HTMLElement>("#history-empty")!;
  private readonly more = document.querySelector<HTMLButtonElement>("#history-more")!;
  private returnFocus: HTMLElement | null = null;

  constructor(private readonly handlers: HistoryHandlers) {
    document.querySelector<HTMLButtonElement>("#history-close")!.addEventListener("click", () => this.close());
    this.more.addEventListener("click", () => this.loadMore());
    this.root.addEventListener("click", (event) => {
      if (event.target === this.root) this.close();
    });
    this.root.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        this.close();
      }
      // Editor shortcuts must not act on the capture hidden behind the overlay.
      event.stopPropagation();
    });
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  async show(): Promise<void> {
    this.list.replaceChildren();
    if (!(await this.loadMore())) return;
    this.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    this.root.hidden = false;
    document.querySelector<HTMLButtonElement>("#history-close")!.focus();
  }

  /** Appends the next page; the offset is what is shown, so deletions never skip entries. */
  private async loadMore(): Promise<boolean> {
    let page: HistoryPage;
    try {
      page = await invoke<HistoryPage>("list_history", { offset: this.list.childElementCount });
    } catch (error) {
      this.handlers.notify(`No se pudo leer el historial: ${error}`, true);
      return false;
    }
    this.list.append(...page.items.map((item) => this.entry(item)));
    this.empty.hidden = this.list.childElementCount > 0;
    this.more.hidden = !page.hasMore;
    return true;
  }

  close(): void {
    if (this.root.hidden) return;
    this.root.hidden = true;
    this.list.replaceChildren();
    this.returnFocus?.focus();
  }

  private entry(item: HistoryItem): HTMLLIElement {
    const row = document.createElement("li");
    row.className = "history-item";

    const open = document.createElement("button");
    open.type = "button";
    open.className = "history-open";
    const date = DATE_FORMAT.format(new Date(item.createdAt));
    open.title = `Abrir la captura del ${date}`;
    const thumbnail = new Image();
    thumbnail.loading = "lazy";
    thumbnail.decoding = "async";
    thumbnail.alt = "";
    thumbnail.src = thumbnailUrl(item.id);
    const label = document.createElement("span");
    label.textContent = date;
    open.append(thumbnail, label);
    open.addEventListener("click", () => this.open(item.id));

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "secondary compact";
    remove.textContent = "Borrar";
    remove.title = `Borrar la captura del ${date}`;
    remove.addEventListener("click", () => this.remove(item.id, row));

    row.append(open, remove);
    return row;
  }

  private async open(id: string): Promise<void> {
    this.close();
    try {
      await this.handlers.open(id);
    } catch (error) {
      this.handlers.notify(`No se pudo abrir la captura: ${error}`, true);
    }
  }

  private async remove(id: string, row: HTMLElement): Promise<void> {
    try {
      await invoke("delete_history", { id });
    } catch (error) {
      this.handlers.notify(`No se pudo borrar la captura: ${error}`, true);
      return;
    }
    row.remove();
    this.empty.hidden = this.list.childElementCount > 0;
    this.handlers.deleted(id);
  }
}
