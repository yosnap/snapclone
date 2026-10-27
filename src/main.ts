import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { bindControls } from "./controls";
import { copyToClipboard, saveToFile } from "./export";
import { type Capture, render } from "./render";
import { defaultStyle } from "./style";

type CaptureReady = { id: string };

const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const status = document.querySelector<HTMLElement>("#status")!;
const copyButton = document.querySelector<HTMLButtonElement>("#copy-button")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-button")!;

const style = defaultStyle();
let capture: Capture | null = null;
let renderQueued = false;

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

async function loadCapture(id: string): Promise<void> {
  const image = new Image();
  // Served with CORS so the canvas is not tainted and can be exported.
  image.crossOrigin = "anonymous";
  image.src = captureUrl(id);
  await image.decode();

  // screencapture writes physical pixels; the window's backing scale converts points to pixels.
  capture = { image, scale: window.devicePixelRatio || 1 };
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

async function copy(): Promise<void> {
  if (!capture) return;
  try {
    await copyToClipboard(canvas);
    showStatus("Copiada al portapapeles");
  } catch (error) {
    showStatus(`No se pudo copiar: ${error}`, true);
  }
}

async function saveImage(): Promise<void> {
  if (!capture) return;
  try {
    if (await saveToFile(canvas)) showStatus("Imagen guardada");
  } catch (error) {
    showStatus(`No se pudo guardar: ${error}`, true);
  }
}

function startCapture(): void {
  invoke("start_capture").catch((error) => showStatus(`No se pudo capturar: ${error}`, true));
}

bindControls(sidebar, style, scheduleRender);

document.querySelectorAll<HTMLButtonElement>("[data-action=capture]").forEach((button) => {
  button.addEventListener("click", startCapture);
});
copyButton.addEventListener("click", copy);
saveButton.addEventListener("click", saveImage);

document.addEventListener("keydown", (event) => {
  if (!event.metaKey || event.shiftKey || event.altKey) return;
  const editingText = event.target instanceof HTMLInputElement && event.target.type === "text";
  if (event.key === "c" && !editingText) {
    event.preventDefault();
    copy();
  } else if (event.key === "s") {
    event.preventDefault();
    saveImage();
  }
});

listen<CaptureReady>("capture-ready", (event) => {
  loadCapture(event.payload.id).catch((error) => showStatus(`No se pudo cargar la captura: ${error}`, true));
});
