import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnnotationEditor, type Tool } from "./annotation-editor";
import { ANNOTATION_COLORS, type PixelateAnnotation, type Point, moved } from "./annotations";
import {
  type RedactCategory,
  type SensitiveRegion,
  bindRedactControls,
  defaultRedactSettings,
  detectSensitive,
  toPixelateAnnotations,
} from "./auto-redact";
import { type BalanceResult, analyzeBalance, balancedCanvas, readPixels } from "./balance";
import { type CaptureDocument, parseCaptureDocument } from "./capture-document";
import { bindControls } from "./controls";
import { DocumentSaver } from "./document-saver";
import { copyToClipboard, saveToFile } from "./export";
import { HistoryView, type OpenedCapture, captureUrl } from "./history-view";
import { type StoredStyle, applyStoredStyle, toStoredStyle } from "./persisted-style";
import { bindPresets } from "./presets";
import { type Capture, layout, makeCapture, render } from "./render";
import { loadSettings, saveSetting } from "./settings";
import { SettingsView } from "./settings-view";
import { type ShortcutEntry, loadShortcuts } from "./shortcut-settings";
import { defaultStyle } from "./style";
import { ensureWallpaper } from "./wallpaper";

type CaptureReady = { id: string; scale: number };

const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const status = document.querySelector<HTMLElement>("#status")!;
const copyButton = document.querySelector<HTMLButtonElement>("#copy-button")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-button")!;
const toolbar = document.querySelector<HTMLElement>("#toolbar")!;
const textEntry = document.querySelector<HTMLInputElement>("#text-entry")!;
const balanceToggle = document.querySelector<HTMLInputElement>("#balance-toggle")!;
const historyButton = document.querySelector<HTMLButtonElement>("#history-button")!;
const settingsButton = document.querySelector<HTMLButtonElement>("#settings-button")!;

const style = defaultStyle();
const redactSettings = defaultRedactSettings();
/** What the editor shows: the capture as taken, or its balanced version. */
let capture: Capture | null = null;
/** The capture as taken; detection boxes and balancing always refer to it. */
let original: Capture | null = null;
let captureId: string | null = null;
/** Balancing is computed once per capture and reused when toggled back on. */
let balanceCache: { for: Capture; result: BalanceResult; capture: Capture | null } | null = null;
/** How far the content currently sits from where it is in `original`. */
let shift: Point = { x: 0, y: 0 };
/**
 * Every region found in the current capture, in `original` coordinates and with stable ids
 * so re-enabling never duplicates. They are shifted like everything else when added.
 */
let detectedRegions: PixelateAnnotation[] = [];
let detectedFor: string | null = null;
/** Capture whose text is being analysed; exports wait for it so nothing leaves unredacted. */
let detectingFor: string | null = null;
const SEARCHING_MESSAGE = "Buscando datos sensibles…";
let renderQueued = false;
let exporting = false;
/**
 * The global defaults new captures start with. They only change when the user touches a
 * control; opening a capture from the history shows its own style without touching them.
 */
const defaults: { style: StoredStyle; balance: boolean } = { style: toStoredStyle(style), balance: false };
/** Redraws the sidebar from `style`; replaced once the controls are bound. */
let refreshControls = () => {};

const documentSaver = new DocumentSaver((id, error) => {
  console.error("capture document could not be saved", error);
  if (id === captureId) showStatus(`No se pudo guardar la edición en el historial: ${error}`, true);
});

const editor = new AnnotationEditor(
  canvas,
  textEntry,
  () =>
    capture
      ? {
          layout: layout(capture, style),
          scale: capture.scale,
          imageWidth: capture.width,
          imageHeight: capture.height,
        }
      : null,
  () => {
    syncToolbar();
    scheduleRender();
    scheduleDocumentSave();
  },
);

function scheduleDocumentSave(): void {
  if (!captureId) return;
  documentSaver.schedule(captureId, currentDocument);
}

function currentDocument(): CaptureDocument {
  return {
    version: 1,
    style: toStoredStyle(style),
    balance: balanceToggle.checked,
    shift: { ...shift },
    annotations: editor.snapshot(),
  };
}

function scheduleRender(): void {
  if (!capture || renderQueued) return;
  if (style.background.kind === "wallpaper") ensureWallpaper(wallpaperLoaded);
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (!capture) return;
    render(canvas, capture, style, editor.overlay(true));
    editor.syncTextEntry();
  });
}

/** The default gradient stands in when macOS gives no usable desktop picture. */
function wallpaperLoaded(error: string | null): void {
  if (error === null) scheduleRender();
  else showStatus(`No se pudo usar el fondo de escritorio (${error}); se muestra el degradado por defecto`, true);
}

/** Opens a capture; `saved` restores the style and annotations it had in the history. */
async function loadCapture({ id, scale }: CaptureReady, saved: CaptureDocument | null = null): Promise<void> {
  const image = new Image();
  // Served with CORS so the canvas is not tainted and can be exported.
  image.crossOrigin = "anonymous";
  image.src = captureUrl(id);
  await image.decode();

  // The previous capture's pending edits are built from the current state: save them first.
  void documentSaver.flush();
  original = makeCapture(image, scale);
  capture = original;
  captureId = id;
  shift = { x: 0, y: 0 };
  detectedRegions = [];
  detectedFor = null;
  editor.reset();
  emptyState.hidden = true;
  canvas.hidden = false;
  showStatus("");
  syncExportButtons();
  // The document's own style, or the defaults for a new capture; neither is persisted here.
  applyStoredStyle(style, saved?.style ?? defaults.style);
  balanceToggle.checked = saved?.balance ?? defaults.balance;
  refreshControls();
  if (balanceToggle.checked) applyBalance(true);
  if (saved) {
    // Saved annotations sit where the content was then; follow it if balancing moved it.
    editor.load(saved.annotations);
    editor.translate(shift.x - saved.shift.x, shift.y - saved.shift.y);
  }
  // Skip Vision entirely while every category is off; enabling one runs it on demand.
  if (Object.values(redactSettings).some(Boolean)) runAutoRedaction(id);
  scheduleRender();
}

/**
 * Reopens a capture from the history. The one already open is left alone, since the
 * database may be behind its latest edits; any other waits until pending edits are stored.
 */
async function reopenCapture(id: string): Promise<void> {
  if (id === captureId) return;
  await documentSaver.flush();
  const opened = await invoke<OpenedCapture>("open_history", { id });
  await loadCapture({ id: opened.id, scale: opened.scale }, parseCaptureDocument(opened.document));
}

/** A capture deleted from the history while open leaves the editor empty. */
function captureDeleted(id: string): void {
  documentSaver.forget(id);
  if (id !== captureId) return;
  capture = null;
  original = null;
  captureId = null;
  balanceCache = null;
  detectedRegions = [];
  detectedFor = null;
  editor.reset();
  canvas.hidden = true;
  emptyState.hidden = false;
  syncExportButtons();
}

function balanceOf(source: Capture): { result: BalanceResult; capture: Capture | null } {
  if (balanceCache?.for !== source) {
    const result = analyzeBalance(readPixels(source.image, source.width, source.height));
    const balanced = result.kind === "balanced" ? makeCapture(balancedCanvas(source.image, result.plan), source.scale) : null;
    balanceCache = { for: source, result, capture: balanced };
  }
  return balanceCache;
}

/**
 * Switches between the original and the balanced capture. Annotations, every history
 * snapshot and the detected regions move with the content so they stay on what they mark.
 */
function applyBalance(enabled: boolean): void {
  if (!original) return;
  let target = original;
  let offset: Point = { x: 0, y: 0 };
  if (enabled) {
    let balance: ReturnType<typeof balanceOf>;
    try {
      balance = balanceOf(original);
    } catch (error) {
      showStatus(`No se pudo equilibrar la captura: ${error}`, true);
      return;
    }
    const { result, capture: balanced } = balance;
    if (result.kind === "balanced" && balanced) {
      target = balanced;
      offset = result.plan.offset;
    } else {
      showStatus(
        result.kind === "not-uniform"
          ? "No se puede equilibrar: el borde de la captura no es de un color uniforme"
          : "No se puede equilibrar: la captura es de un solo color",
      );
    }
  }
  editor.translate(offset.x - shift.x, offset.y - shift.y);
  shift = offset;
  capture = target;
  scheduleRender();
  scheduleDocumentSave();
}

/** Detected regions are kept in original coordinates; this places them on the current capture. */
function shifted(regions: PixelateAnnotation[]): PixelateAnnotation[] {
  return regions.map((region) => moved(region, shift.x, shift.y) as PixelateAnnotation);
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
    if (id !== captureId || !original) return;
    detectedRegions = toPixelateAnnotations(regions, original.width, original.height, original.scale);
    detectedFor = id;
    const enabled = detectedRegions.filter((region) => region.auto && redactSettings[region.auto]);
    editor.addAutoRedactions(shifted(enabled));
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
    editor.addAutoRedactions(shifted(regionsOf(category)));
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

/** Every style control and preset goes through here: these are the user's choices. */
function styleChanged(): void {
  scheduleRender();
  defaults.style = toStoredStyle(style);
  saveSetting("style", defaults.style);
  scheduleDocumentSave();
}

/** Restores the last session's choices before wiring the controls, so they start in sync. */
async function init(): Promise<void> {
  const settings = await loadSettings();
  defaults.style = settings.style;
  defaults.balance = settings.balance;
  applyStoredStyle(style, settings.style);
  Object.assign(redactSettings, settings.redact);
  editor.setColor(settings.annotationColor);

  refreshControls = bindControls(sidebar, style, styleChanged);
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
  balanceToggle.checked = settings.balance;
  balanceToggle.addEventListener("change", () => {
    defaults.balance = balanceToggle.checked;
    saveSetting("balance", balanceToggle.checked);
    applyBalance(balanceToggle.checked);
  });
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

const historyView = new HistoryView({
  open: reopenCapture,
  deleted: captureDeleted,
  notify: showStatus,
});
historyButton.addEventListener("click", () => historyView.show());

/** The empty editor names the zone shortcut as it is configured right now. */
function showZoneShortcut(entries: ShortcutEntry[]): void {
  const hint = document.querySelector<HTMLElement>("#empty-hint")!;
  const zone = entries.find((entry) => entry.kind === "region");
  if (!zone?.label) {
    hint.textContent = "La captura de zona no tiene atajo: usa el botón o el icono de la barra de menús.";
    return;
  }
  const key = document.createElement("kbd");
  key.textContent = zone.label;
  hint.replaceChildren("Pulsa ", key, " o usa el icono de la barra de menús.");
}

const settingsView = new SettingsView(showZoneShortcut);
function openSettings(): void {
  historyView.close();
  settingsView.show().catch((error) => showStatus(`No se pudieron abrir los ajustes: ${error}`, true));
}
settingsButton.addEventListener("click", openSettings);

document.addEventListener("keydown", (event) => {
  if (historyView.isOpen) {
    if (event.key === "Escape") historyView.close();
    return;
  }
  if (settingsView.isOpen) return;
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
  loadShortcuts()
    .then(showZoneShortcut)
    .catch((error) => console.error("shortcuts could not be read", error));
  listen<CaptureReady>("capture-ready", (event) => {
    historyView.close();
    settingsView.close();
    loadCapture(event.payload).catch((error) => showStatus(`No se pudo cargar la captura: ${error}`, true));
  });
  // "Ajustes…" in the app menu (⌘,) and in the menu bar icon.
  listen("open-settings", openSettings);
  listen<string>("capture-failed", (event) => {
    historyView.close();
    showStatus(`No se pudo hacer la captura: ${event.payload}`, true);
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
