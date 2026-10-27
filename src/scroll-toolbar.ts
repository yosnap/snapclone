import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The page of the scroll capture windows. The four frame windows only show a colour; the
 * toolbar shows the stitched height and ends the capture with Terminar (Enter) or
 * Cancelar (Esc).
 */
type Progress = { height: number; maxHeight: number; state: "capturing" | "lost" | "full" | "failed" };

const NOTES: Record<Progress["state"], { text: string; warning: boolean }> = {
  capturing: { text: "Desplázate por la zona marcada", warning: false },
  lost: { text: "No encuentro dónde encaja: desplázate más despacio", warning: true },
  full: { text: "", warning: true },
  failed: { text: "No se pudo capturar la zona", warning: true },
};

const heightLabel = document.querySelector<HTMLElement>("#scroll-height")!;
const note = document.querySelector<HTMLElement>("#scroll-note")!;
const finishButton = document.querySelector<HTMLButtonElement>("#scroll-finish")!;
const cancelButton = document.querySelector<HTMLButtonElement>("#scroll-cancel")!;
const NUMBER = new Intl.NumberFormat("es-ES");
/** Set once a result was sent, so nothing is sent twice while the windows close. */
let finished = false;

function end(command: "finish_scroll" | "cancel_scroll"): void {
  if (finished) return;
  finished = true;
  finishButton.disabled = true;
  cancelButton.disabled = true;
  invoke(command).catch((error) => {
    // Rust shows a failed capture in the editor; the windows are closing by now.
    console.error(`${command} failed`, error);
  });
}

function showProgress({ height, maxHeight, state }: Progress): void {
  heightLabel.textContent = `Altura: ${NUMBER.format(height)} px`;
  const { text, warning } = NOTES[state];
  note.textContent = state === "full" ? `Altura máxima alcanzada (${NUMBER.format(maxHeight)} px)` : text;
  note.classList.toggle("warning", warning);
  finishButton.disabled = finished || state === "failed";
}

if (getCurrentWindow().label.startsWith("scroll-edge-")) {
  document.body.classList.add("edge");
} else {
  finishButton.addEventListener("click", () => end("finish_scroll"));
  cancelButton.addEventListener("click", () => end("cancel_scroll"));
  document.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      end("finish_scroll");
    } else if (event.key === "Escape") {
      event.preventDefault();
      end("cancel_scroll");
    }
  });
  document.addEventListener("contextmenu", (event) => event.preventDefault());
  listen<Progress>("scroll-progress", (event) => showProgress(event.payload)).catch((error) => {
    console.error("scroll progress unavailable", error);
    note.textContent = "No se puede mostrar el progreso";
  });
}
