import { invoke } from "@tauri-apps/api/core";
import { shortcutFromEvent } from "./shortcut-keys";

type CaptureKind = "region" | "window" | "screen" | "scroll";
/**
 * As Rust reports it: `name` is the capture kind as the user reads it, `label` the shortcut
 * as macOS shows it and `error` a failed registration.
 */
export type ShortcutEntry = {
  kind: CaptureKind;
  name: string;
  shortcut: string | null;
  label: string | null;
  error: string | null;
};

export function loadShortcuts(): Promise<ShortcutEntry[]> {
  return invoke<ShortcutEntry[]>("get_shortcuts");
}
const RECORDING_TEXT = "Pulsa la combinación…";
const EMPTY_TEXT = "Sin atajo";

/**
 * One row per capture kind with a shortcut recorder. Recording pauses every global shortcut
 * in Rust, so pressing one of them reaches the recorder instead of starting a capture.
 * Validation and registration happen in Rust; its error is shown on the row.
 */
export class ShortcutSettings {
  private entries: ShortcutEntry[] = [];
  /** Errors from the last change attempt, which Rust does not keep. */
  private readonly attemptErrors = new Map<CaptureKind, string>();
  private recording: { kind: CaptureKind; button: HTMLButtonElement } | null = null;

  /** `changed` hears about every shortcut list shown, so other views can follow it. */
  constructor(
    private readonly list: HTMLUListElement,
    private readonly changed: (entries: ShortcutEntry[]) => void,
  ) {}

  async load(): Promise<void> {
    this.attemptErrors.clear();
    this.render(await loadShortcuts());
  }

  /** Ends any recording without changing anything, e.g. when the settings close. */
  stop(): void {
    if (this.recording) this.cancel();
  }

  private render(entries: ShortcutEntry[]): void {
    this.entries = entries;
    this.changed(entries);
    this.recording = null;
    this.list.replaceChildren(...entries.map((entry) => this.row(entry)));
  }

  private row(entry: ShortcutEntry): HTMLLIElement {
    const row = document.createElement("li");
    row.className = "shortcut-row";

    const name = document.createElement("span");
    name.textContent = entry.name;
    name.id = `shortcut-${entry.kind}`;

    const recorder = document.createElement("button");
    recorder.type = "button";
    recorder.className = "shortcut-recorder";
    recorder.textContent = entry.label ?? EMPTY_TEXT;
    recorder.setAttribute("aria-pressed", "false");
    recorder.setAttribute("aria-labelledby", name.id);
    recorder.title = "Haz clic y pulsa la nueva combinación";
    recorder.addEventListener("click", () => this.start(entry.kind, recorder));
    recorder.addEventListener("keydown", (event) => this.onKey(event, entry.kind));
    recorder.addEventListener("blur", () => {
      if (this.recording?.button === recorder) this.cancel();
    });

    const clear = document.createElement("button");
    clear.type = "button";
    clear.className = "secondary compact";
    clear.textContent = "Borrar";
    clear.title = `Quitar el atajo de «${entry.name}»`;
    clear.disabled = entry.shortcut === null;
    clear.addEventListener("click", () => this.assign(entry.kind, null));

    const error = document.createElement("p");
    error.className = "shortcut-error";
    error.textContent = this.attemptErrors.get(entry.kind) ?? entry.error ?? "";
    error.hidden = !error.textContent;

    row.append(name, recorder, clear, error);
    return row;
  }

  private start(kind: CaptureKind, button: HTMLButtonElement): void {
    if (this.recording?.button === button) return;
    this.stop();
    this.recording = { kind, button };
    button.textContent = RECORDING_TEXT;
    button.setAttribute("aria-pressed", "true");
    invoke("pause_shortcuts", { paused: true }).catch((error) => {
      // The recorder still works; a global shortcut pressed now may start a capture instead.
      console.error("global shortcuts could not be paused", error);
    });
  }

  private onKey(event: KeyboardEvent, kind: CaptureKind): void {
    if (this.recording?.kind !== kind) return;
    event.preventDefault();
    event.stopPropagation();
    const modified = event.metaKey || event.ctrlKey || event.altKey || event.shiftKey;
    if (event.key === "Escape" && !modified) {
      this.cancel();
      return;
    }
    const shortcut = shortcutFromEvent(event);
    if (shortcut) void this.assign(kind, shortcut);
  }

  private cancel(): void {
    const recording = this.recording;
    this.recording = null;
    if (recording) {
      const entry = this.entries.find((item) => item.kind === recording.kind);
      recording.button.textContent = entry?.label ?? EMPTY_TEXT;
      recording.button.setAttribute("aria-pressed", "false");
    }
    resumeShortcuts();
  }

  /** Saves a new shortcut (or clears it with `null`); on failure the previous one is kept. */
  private async assign(kind: CaptureKind, shortcut: string | null): Promise<void> {
    this.recording = null;
    try {
      const entries = await invoke<ShortcutEntry[]>("set_shortcut", { kind, shortcut });
      this.attemptErrors.delete(kind);
      this.render(entries);
    } catch (error) {
      this.attemptErrors.set(kind, String(error));
      try {
        this.render(await loadShortcuts());
      } catch (reloadError) {
        console.error("shortcuts could not be reloaded", reloadError);
        this.render(this.entries);
      }
    } finally {
      // Rust resumes on success; after a rejected combination the pause is still on.
      resumeShortcuts();
    }
  }
}

function resumeShortcuts(): void {
  invoke("pause_shortcuts", { paused: false }).catch((error) => console.error("global shortcuts could not be resumed", error));
}
