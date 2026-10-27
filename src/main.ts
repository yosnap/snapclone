import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { bindControls } from "./controls";
import { copyToClipboard, saveToFile } from "./export";
import { type Capture, render } from "./render";
import { defaultStyle } from "./style";

type CaptureReady = { id: string; scale: number };

const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const status = document.querySelector<HTMLElement>("#status")!;
const copyButton = document.querySelector<HTMLButtonElement>("#copy-button")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-button")!;

const style = defaultStyle();
let capture: Capture | null = null;
let renderQueued = false;
let exporting = false;

function scheduleRender(): void {
  if (!capture || renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (capture) render(canvas, capture, style);
  });
}

function captureUrl(id: string): string {
  return `capture://localhost/${id}`;
}

async function loadCapture({ id, scale }: CaptureReady): Promise<void> {
  const image = new Image();
  // Served with CORS so the canvas is not tainted and can be exported.
  image.crossOrigin = "anonymous";
  image.src = captureUrl(id);
  await image.decode();

  capture = { image, scale };
  emptyState.hidden = true;
  canvas.hidden = false;
  copyButton.disabled = false;
  saveButton.disabled = false;
  showStatus("");
  scheduleRender();
}

function showStatus(message: string, isError = false): void {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

/** Renders synchronously before exporting, so a pending frame never leaks into the output. */
async function exportWith(action: (canvas: HTMLCanvasElement) => Promise<string | null>, failure: string) {
  if (!capture || exporting) return;
  exporting = true;
  try {
    render(canvas, capture, style);
    const message = await action(canvas);
    if (message) showStatus(message);
  } catch (error) {
    showStatus(`${failure}: ${error}`, true);
  } finally {
    exporting = false;
  }
}

const copy = () =>
  exportWith(async (target) => {
    await copyToClipboard(target);
    return "Copiada al portapapeles";
  }, "No se pudo copiar");

const saveImage = () =>
  exportWith(async (target) => ((await saveToFile(target)) ? "Imagen guardada" : null), "No se pudo guardar");

function startCapture(): void {
  if (outsideTauri) return;
  invoke("start_capture").catch((error) => showStatus(`No se pudo capturar: ${error}`, true));
}

bindControls(sidebar, style, scheduleRender);

// The Vite dev server is reachable from any browser, but only the Tauri window has the Rust bridge.
const outsideTauri = !isTauri();
if (outsideTauri) {
  document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => (button.disabled = true));
  showStatus("Esto es la vista web en un navegador. Usa la ventana de Snapclone que abre «npm run tauri dev».", true);
}

document.querySelectorAll<HTMLButtonElement>("[data-action=capture]").forEach((button) => {
  button.addEventListener("click", startCapture);
});
copyButton.addEventListener("click", copy);
saveButton.addEventListener("click", saveImage);

document.addEventListener("keydown", (event) => {
  if (!event.metaKey || event.shiftKey || event.altKey || event.repeat) return;
  const editingText = event.target instanceof HTMLInputElement && event.target.type === "text";
  if (event.key === "c" && !editingText) {
    event.preventDefault();
    copy();
  } else if (event.key === "s") {
    event.preventDefault();
    saveImage();
  }
});

if (!outsideTauri) {
  listen<CaptureReady>("capture-ready", (event) => {
    loadCapture(event.payload).catch((error) => showStatus(`No se pudo cargar la captura: ${error}`, true));
  });
}
