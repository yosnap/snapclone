import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnnotationEditor, type Tool } from "./annotation-editor";
import { ANNOTATION_COLORS, type PixelateAnnotation } from "./annotations";
import {
  type RedactCategory,
  type SensitiveRegion,
  bindRedactControls,
  defaultRedactSettings,
  detectSensitive,
  toPixelateAnnotations,
} from "./auto-redact";
import { bindControls } from "./controls";
import { copyToClipboard, saveToFile } from "./export";
import { applyStoredStyle, toStoredStyle } from "./persisted-style";
import { bindPresets } from "./presets";
import { type Capture, layout, render } from "./render";
import { loadSettings, saveSetting } from "./settings";
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
const redactSettings = defaultRedactSettings();
let capture: Capture | null = null;
let captureId: string | null = null;
/** Every region found in the current capture, with stable ids so re-enabling never duplicates. */
let detectedRegions: PixelateAnnotation[] = [];
let detectedFor: string | null = null;
/** Capture whose text is being analysed; exports wait for it so nothing leaves unredacted. */
let detectingFor: string | null = null;
const SEARCHING_MESSAGE = "Buscando datos sensibles…";
let renderQueued = false;
let exporting = false;

const editor = new AnnotationEditor(
  canvas,
  textEntry,
  () =>
    capture
      ? {
          layout: layout(capture, style),
          scale: capture.scale,
          imageWidth: capture.image.naturalWidth,
          imageHeight: capture.image.naturalHeight,
        }
      : null,
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
    if (!capture) return;
    render(canvas, capture, style, editor.overlay(true));
    editor.syncTextEntry();
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
  captureId = id;
  detectedRegions = [];
  detectedFor = null;
  editor.reset();
  emptyState.hidden = true;
  canvas.hidden = false;
  showStatus("");
  syncExportButtons();
  scheduleRender();
  // Skip Vision entirely while every category is off; enabling one runs it on demand.
  if (Object.values(redactSettings).some(Boolean)) runAutoRedaction(id);
}

function syncExportButtons(): void {
  const blocked = !capture || detectingFor === captureId;
  copyButton.disabled = blocked;
  saveButton.disabled = blocked;
}

function regionsOf(category: RedactCategory): PixelateAnnotation[] {
  return detectedRegions.filter((region) => region.auto === category);
}

async function runAutoRedaction(id: string): Promise<void> {
  if (detectedFor === id || detectingFor === id) return;
  detectingFor = id;
  syncExportButtons();
  showStatus(SEARCHING_MESSAGE);
  try {
    const regions: SensitiveRegion[] = await detectSensitive(id);
    // A newer capture may have replaced this one while Vision was running.
    if (id !== captureId || !capture) return;
    detectedRegions = toPixelateAnnotations(regions, capture.image.naturalWidth, capture.image.naturalHeight, capture.scale);
    detectedFor = id;
    const enabled = detectedRegions.filter((region) => region.auto && redactSettings[region.auto]);
    editor.addAutoRedactions(enabled);
    // Only replace our own progress message, never an export confirmation shown meanwhile.
    if (status.textContent === SEARCHING_MESSAGE) {
      showStatus(enabled.length > 0 ? `Tachados automáticamente: ${enabled.length}` : "");
    }
  } catch (error) {
    if (id === captureId) showStatus(`No se pudo analizar el texto, revisa la captura antes de compartirla: ${error}`, true);
  } finally {
    if (detectingFor === id) detectingFor = null;
    syncExportButtons();
  }
}

function toggleRedaction(category: RedactCategory, enabled: boolean): void {
  saveSetting("redact", { ...redactSettings });
  if (!capture || !captureId) return;
  if (!enabled) {
    editor.removeAutoRedactions(category);
  } else if (detectedFor === captureId) {
    editor.addAutoRedactions(regionsOf(category));
  } else {
    runAutoRedaction(captureId);
  }
}

function showStatus(message: string, isError = false): void {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

/** Renders synchronously before exporting, so a pending frame never leaks into the output. */
async function exportWith(action: (canvas: HTMLCanvasElement) => Promise<string | null>, failure: string) {
  if (!capture || exporting) return;
  if (detectingFor === captureId) {
    showStatus("Espera a que termine la búsqueda de datos sensibles", true);
    return;
  }
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

function styleChanged(): void {
  scheduleRender();
  saveSetting("style", toStoredStyle(style));
}

/** Restores the last session's choices before wiring the controls, so they start in sync. */
async function init(): Promise<void> {
  const settings = await loadSettings();
  applyStoredStyle(style, settings.style);
  Object.assign(redactSettings, settings.redact);
  editor.setColor(settings.annotationColor);

  const refreshControls = bindControls(sidebar, style, styleChanged);
  bindPresets(sidebar.querySelector<HTMLElement>("#presets")!, settings.presets, {
    current: () => toStoredStyle(style),
    apply: (stored) => {
      applyStoredStyle(style, stored);
      refreshControls();
      styleChanged();
    },
    persist: (presets) => saveSetting("presets", presets),
    notify: (message) => showStatus(message),
  });
  bindRedactControls(sidebar.querySelector<HTMLElement>("#redact-options")!, redactSettings, toggleRedaction);
  buildToolbar();

  // The Vite dev server is reachable from any browser, but only the Tauri window has the Rust bridge.
  if (outsideTauri) {
    document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => (button.disabled = true));
    showStatus("Esto es la vista web en un navegador. Usa la ventana de Snapclone que abre «npm run tauri dev».", true);
  }
}

const outsideTauri = !isTauri();
init().catch((error) => showStatus(`No se pudo iniciar el editor: ${error}`, true));

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
    else if (button.dataset.color) {
      editor.setColor(button.dataset.color);
      saveSetting("annotationColor", editor.color);
    }
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
