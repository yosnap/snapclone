import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnnotationEditor, type Tool } from "./annotation-editor";
import { ANNOTATION_COLORS } from "./annotations";
import { bindControls } from "./controls";
import { copyToClipboard, saveToFile } from "./export";
import { type Capture, layout, render } from "./render";
import { defaultStyle } from "./style";

type CaptureReady = { id: string; scale: number };

const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const status = document.querySelector<HTMLElement>("#status")!;
const copyButton = document.querySelector<HTMLButtonElement>("#copy-button")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-button")!;
const toolbar = document.querySelector<HTMLElement>("#toolbar")!;
const textEntry = document.querySelector<HTMLInputElement>("#text-entry")!;

const style = defaultStyle();
let capture: Capture | null = null;
let renderQueued = false;
let exporting = false;

const editor = new AnnotationEditor(
  canvas,
  textEntry,
  () => (capture ? { layout: layout(capture, style), scale: capture.scale } : null),
  () => {
    syncToolbar();
    scheduleRender();
  },
);

function scheduleRender(): void {
  if (!capture || renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (capture) render(canvas, capture, style, editor.overlay(true));
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
  editor.reset();
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
    render(canvas, capture, style, editor.overlay(false));
    const message = await action(canvas);
    if (message) showStatus(message);
  } catch (error) {
    showStatus(`${failure}: ${error}`, true);
  } finally {
    exporting = false;
    scheduleRender();
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
buildToolbar();

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
  const editingText = event.target instanceof HTMLInputElement && event.target.type === "text";
  const inForm = editingText || event.target instanceof HTMLSelectElement;
  if (capture && !inForm && editor.handleKey(event)) {
    event.preventDefault();
    return;
  }
  if (!event.metaKey || event.shiftKey || event.altKey || event.repeat) return;
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

function buildToolbar(): void {
  const colors = toolbar.querySelector<HTMLElement>("#annotation-colors")!;
  for (const color of ANNOTATION_COLORS) {
    const swatch = document.createElement("button");
    swatch.type = "button";
    swatch.className = "color-swatch";
    swatch.dataset.color = color;
    swatch.title = color;
    swatch.style.background = color;
    colors.append(swatch);
  }
  toolbar.addEventListener("click", (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
    if (!button) return;
    if (button.dataset.tool) editor.setTool(button.dataset.tool as Tool);
    else if (button.dataset.color) editor.setColor(button.dataset.color);
    else if (button.dataset.history === "undo") editor.undo();
    else if (button.dataset.history === "redo") editor.redo();
    syncToolbar();
  });
  editor.setTool("select");
}

function syncToolbar(): void {
  toolbar.querySelectorAll<HTMLButtonElement>("[data-tool]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.tool === editor.tool));
  });
  toolbar.querySelectorAll<HTMLButtonElement>("[data-color]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.color === editor.color));
  });
}
