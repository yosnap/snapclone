import { invoke } from "@tauri-apps/api/core";

/**
 * In-window settings overlay. These settings belong to the app rather than the editor, so
 * they are read from and written through Rust, which applies them at once.
 */
export class SettingsView {
  private readonly root = document.querySelector<HTMLElement>("#settings")!;
  private readonly dockHidden = document.querySelector<HTMLInputElement>("#dock-hidden")!;
  private readonly status = document.querySelector<HTMLElement>("#settings-status")!;
  private returnFocus: HTMLElement | null = null;

  constructor() {
    document.querySelector<HTMLButtonElement>("#settings-close")!.addEventListener("click", () => this.close());
    this.root.addEventListener("click", (event) => {
      if (event.target === this.root) this.close();
    });
    this.root.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        this.close();
      }
      // Editor shortcuts must not act on the capture hidden behind the overlay.
      event.stopPropagation();
    });
    this.dockHidden.addEventListener("change", () => this.setDockHidden(this.dockHidden.checked));
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  async show(): Promise<void> {
    if (this.isOpen) return;
    this.showStatus("");
    try {
      this.dockHidden.checked = await invoke<boolean>("get_dock_hidden");
    } catch (error) {
      this.showStatus(`No se pudieron leer los ajustes: ${error}`, true);
    }
    this.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    this.root.hidden = false;
    document.querySelector<HTMLButtonElement>("#settings-close")!.focus();
  }

  close(): void {
    if (this.root.hidden) return;
    this.root.hidden = true;
    this.returnFocus?.focus();
  }

  private async setDockHidden(hidden: boolean): Promise<void> {
    try {
      await invoke("set_dock_hidden", { hidden });
      this.showStatus("");
    } catch (error) {
      this.dockHidden.checked = !hidden;
      this.showStatus(`No se pudo cambiar el icono del Dock: ${error}`, true);
    }
  }

  private showStatus(message: string, isError = false): void {
    this.status.textContent = message;
    this.status.classList.toggle("error", isError);
  }
}
