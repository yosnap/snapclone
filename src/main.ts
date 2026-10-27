import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnnotationEditor } from "./annotation-editor";
import { type PixelateAnnotation, type Point, moved } from "./annotations";
import {
  REDACT_CATEGORIES,
  type RedactCategory,
  type RedactMode,
  type SensitiveRegion,
  defaultRedactSettings,
  detectSensitive,
  toPixelateAnnotations,
} from "./auto-redact";
import { type BalanceResult, analyzeBalance, balancedCanvas, readPixels } from "./balance";
import { type CaptureDocument, parseCaptureDocument } from "./capture-document";
import { DocumentSaver } from "./document-saver";
import { BottomBar } from "./bottom-bar";
import { copyToClipboard, prepareExport, quickSave, saveToFile } from "./export";
import { HistoryView, type OpenedCapture, captureUrl } from "./history-view";
import { fillIcons } from "./icons";
import { bindInspector } from "./inspector";
import { highlightDrops, listenNotices, openClipboard, openFile } from "./open-image";
import { type StoredStyle, applyStoredStyle, toStoredStyle } from "./persisted-style";
import { type MenuItem, closePopover, popoverContains } from "./popover";
import { bindPresets } from "./presets";
import { bindRedactPanel } from "./redact-panel";
import { type Capture, edgeColor, layout, makeCapture, outputCanvas, render } from "./render";
import { bindReopen, reopenLast } from "./reopen-last";
import { loadSettings, saveSetting } from "./settings";
import { SettingsView } from "./settings-view";
import { type ShortcutEntry, loadShortcuts } from "./shortcut-settings";
import { defaultStyle } from "./style";
import { TextView } from "./text-view";
import { bindTopToolbar } from "./top-toolbar";
import { isEditableTarget } from "./ui-helpers";
import { requestWallpaper, wallpaperError } from "./wallpaper";
import { Zoom } from "./zoom";

type CaptureReady = { id: string; scale: number };

fillIcons();
const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const canvasArea = document.querySelector<HTMLElement>("#canvas-area")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const status = document.querySelector<HTMLElement>("#status")!;
const titlebar = document.querySelector<HTMLElement>(".titlebar")!;
const shareButton = document.querySelector<HTMLButtonElement>("#share-button")!;
const textEntry = document.querySelector<HTMLInputElement>("#text-entry")!;
const balanceToggle = document.querySelector<HTMLInputElement>("#balance-toggle")!;
/** How long a message stays over the canvas; errors stay longer. */
const STATUS_MS = 3500;
const ERROR_STATUS_MS = 8000;
let statusTimer: number | undefined;

const style = defaultStyle();
const redactSettings = defaultRedactSettings();
/** The main checkbox and the way regions are hidden; the categories are `redactSettings`. */
const redactPreferences = { enabled: true, mode: "pixelate" as RedactMode, categories: redactSettings };
/** Shows how many regions were found (null while searching); replaced once bound. */
let showFound: (found: number | null) => void = () => {};
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

/** Reflects the editor's tool and colour on the title bar; replaced once it is bound. */
let syncToolbar = () => {};

const zoom = new Zoom(canvasArea, canvas, () => capture?.scale ?? null, (label) => bottomBar.setZoomLabel(label));

function scheduleDocumentSave(): void {
  // The development preview has no history to save into.
  if (!captureId || outsideTauri) return;
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
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (!capture) return;
    render(canvas, capture, style, editor.overlay(true));
    canvas.classList.toggle("transparent", style.background.kind === "none");
    zoom.update();
    editor.syncTextEntry();
  });
}

/**
 * Loads the desktop picture, or reloads it if it changed, when the style uses it. Asked for
 * on opening a capture and on picking the swatch; the default gradient stands in until it
 * arrives, or if macOS gives none.
 */
function requireWallpaper(): void {
  if (style.background.kind !== "wallpaper") return;
  requestWallpaper().then(scheduleRender, (error) =>
    showStatus(`No se pudo usar el fondo de escritorio (${wallpaperError(error)}); se muestra el degradado por defecto`, true),
  );
}

/** Opens a capture; `saved` restores the style and annotations it had in the history. */
async function loadCapture({ id, scale }: CaptureReady, saved: CaptureDocument | null = null): Promise<void> {
  const image = new Image();
  // Served with CORS so the canvas is not tainted and can be exported.
  image.crossOrigin = "anonymous";
  image.src = captureUrl(id);
  await image.decode();
  showImage(image, scale, id, saved);
}

function showImage(image: HTMLImageElement, scale: number, id: string, saved: CaptureDocument | null = null): void {
  closePopover();
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
  // A colour picked for the previous capture's inset does not fit this one.
  style.insetColor = null;
  balanceToggle.checked = saved?.balance ?? defaults.balance;
  refreshControls();
  if (balanceToggle.checked) applyBalance(true);
  if (saved) {
    // Saved annotations sit where the content was then; follow it if balancing moved it.
    editor.load(saved.annotations);
    editor.translate(shift.x - saved.shift.x, shift.y - saved.shift.y);
  }
  requireWallpaper();
  // Skip Vision entirely while every category is off; enabling one runs it on demand.
  showFound(0);
  if (!outsideTauri && anyRedaction()) runAutoRedaction(id);
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
  // The inset colour shown is the edge colour of the capture now shown.
  refreshControls();
  scheduleRender();
  scheduleDocumentSave();
}

/** Detected regions are kept in original coordinates; this places them on the current capture. */
function shifted(regions: PixelateAnnotation[]): PixelateAnnotation[] {
  return regions.map((region) => moved(region, shift.x, shift.y) as PixelateAnnotation);
}

function syncExportButtons(): void {
  const blocked = !capture || detectingFor === captureId;
  bottomBar.setExportEnabled(!blocked);
  shareButton.disabled = blocked;
}

function regionsOf(category: RedactCategory): PixelateAnnotation[] {
  return detectedRegions.filter((region) => region.auto === category);
}

function categoryApplies(category: RedactCategory): boolean {
  return redactPreferences.enabled && redactSettings[category];
}

/** Vision is skipped entirely while nothing would be redacted; enabling runs it on demand. */
function anyRedaction(): boolean {
  return redactPreferences.enabled && Object.values(redactSettings).some(Boolean);
}

/** Detected regions placed on the capture shown and hidden the way the user chose. */
function applicable(regions: PixelateAnnotation[]): PixelateAnnotation[] {
  const mode = redactPreferences.mode === "solid" ? { mode: "solid" as const } : {};
  return shifted(regions).map((region) => ({ ...region, ...mode }));
}

function removeAllAutoRedactions(): void {
  for (const { id } of REDACT_CATEGORIES) editor.removeAutoRedactions(id);
}

/** The main checkbox or the mode changed: the regions on the capture follow. */
function refreshAutoRedactions(): void {
  saveSetting("autoRedact", redactPreferences.enabled);
  saveSetting("redactMode", redactPreferences.mode);
  if (!capture || !captureId) return;
  removeAllAutoRedactions();
  if (!anyRedaction()) return;
  if (detectedFor === captureId) {
    editor.addAutoRedactions(applicable(detectedRegions.filter((region) => region.auto && categoryApplies(region.auto))));
  } else {
    runAutoRedaction(captureId);
  }
}

async function runAutoRedaction(id: string): Promise<void> {
  if (detectedFor === id || detectingFor === id) return;
  detectingFor = id;
  syncExportButtons();
  showFound(null);
  showStatus(SEARCHING_MESSAGE);
  try {
    const regions: SensitiveRegion[] = await detectSensitive(id);
    // A newer capture may have replaced this one while Vision was running.
    if (id !== captureId || !original) return;
    detectedRegions = toPixelateAnnotations(regions, original.width, original.height, original.scale);
    detectedFor = id;
    showFound(detectedRegions.length);
    const enabled = detectedRegions.filter((region) => region.auto && categoryApplies(region.auto));
    editor.addAutoRedactions(applicable(enabled));
    // Only replace our own progress message, never an export confirmation shown meanwhile.
    if (status.textContent === SEARCHING_MESSAGE) {
      showStatus(enabled.length > 0 ? `Tachados automáticamente: ${enabled.length}` : "");
    }
  } catch (error) {
    if (id === captureId) showStatus(`No se pudo analizar el texto, revisa la captura antes de compartirla: ${error}`, true);
  } finally {
    if (detectingFor === id) detectingFor = null;
    if (id === captureId && detectedFor !== id) showFound(0);
    syncExportButtons();
  }
}

function toggleRedaction(category: RedactCategory, enabled: boolean): void {
  saveSetting("redact", { ...redactSettings });
  if (!capture || !captureId || !redactPreferences.enabled) return;
  if (!enabled) {
    editor.removeAutoRedactions(category);
  } else if (detectedFor === captureId) {
    editor.addAutoRedactions(applicable(regionsOf(category)));
  } else {
    runAutoRedaction(captureId);
  }
}

function showStatus(message: string, isError = false): void {
  window.clearTimeout(statusTimer);
  status.textContent = message;
  status.classList.toggle("error", isError);
  status.hidden = !message;
  if (message) {
    statusTimer = window.setTimeout(() => {
      // A newer message keeps its own timer.
      if (status.textContent === message && message !== SEARCHING_MESSAGE) status.hidden = true;
    }, isError ? ERROR_STATUS_MS : STATUS_MS);
  }
}

/** Renders synchronously before exporting, so a pending frame never leaks into the output. */
async function exportWith(action: (canvas: HTMLCanvasElement) => Promise<string | null>, failure: string): Promise<boolean> {
  if (!capture || exporting) return false;
  if (detectingFor === captureId) {
    showStatus("Espera a que termine la búsqueda de datos sensibles", true);
    return false;
  }
  exporting = true;
  try {
    // Never export the stand-in gradient: wait for the picture, or say why there is none.
    if (style.background.kind === "wallpaper") {
      try {
        await requestWallpaper();
      } catch (error) {
        showStatus(`No se ha exportado: el fondo de escritorio no está disponible (${wallpaperError(error)})`, true);
        return false;
      }
    }
    render(canvas, capture, style, editor.overlay(false));
    const message = await action(outputCanvas(canvas, style));
    if (message) showStatus(message);
    return true;
  } catch (error) {
    showStatus(`${failure}: ${error}`, true);
    return false;
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

const saveAs = () =>
  exportWith(async (target) => ((await saveToFile(target)) ? "Imagen guardada" : null), "No se pudo guardar");

const save = () =>
  exportWith(async (target) => {
    const path = await quickSave(target);
    return path ? `Guardada en ${path}` : null;
  }, "No se pudo guardar");

/** Double-click on the background: copy, then close the editor like its close button. */
async function copyAndClose(): Promise<void> {
  if (!(await copy())) return;
  invoke("close_editor").catch((error) => showStatus(`No se pudo cerrar el editor: ${error}`, true));
}

/** The image file "Compartir" and "Arrástrame" hand to macOS, written by Rust. */
const prepareShared = () => exportWith(async (target) => (await prepareExport(target), null), "No se pudo preparar la imagen");

async function share(anchor: HTMLElement): Promise<void> {
  if (!(await prepareShared())) return;
  const box = anchor.getBoundingClientRect();
  const position = { x: box.left, y: box.top, width: box.width, height: box.height };
  invoke("share_export", { anchor: position }).catch((error) => showStatus(`No se pudo compartir: ${error}`, true));
}

function copyDetectedText(): void {
  if (!captureId) return;
  invoke<number>("copy_capture_text", { id: captureId }).then(
    (lines) => showStatus(lines === 1 ? "Copiada 1 línea de texto" : `Copiadas ${lines} líneas de texto`),
    (error) => showStatus(String(error), true),
  );
}

function revealSaved(): void {
  invoke("reveal_saved").catch((error) => showStatus(`No se pudo abrir el Finder: ${error}`, true));
}

function moreItems(): MenuItem[] {
  const noCapture = !capture;
  return [
    { label: "Abrir en el Finder", action: revealSaved },
    { label: "Copiar texto detectado", action: copyDetectedText, disabled: noCapture },
    { label: "Ver texto detectado…", action: () => captureId && void textView.show(captureId), disabled: noCapture },
    "separator",
    { label: "Nueva captura", action: startCapture },
    { label: "Abrir archivo…", shortcut: "⌘O", action: () => openFile(showStatus) },
    { label: "Historial", action: () => void historyView.show() },
    "separator",
    { label: "Deshacer", shortcut: "⌘Z", action: () => editor.undo(), disabled: noCapture },
    { label: "Rehacer", shortcut: "⇧⌘Z", action: () => editor.redo(), disabled: noCapture },
    "separator",
    { label: "Ajustes…", shortcut: "⌘,", action: openSettings },
  ];
}

const bottomBar = new BottomBar(zoom, {
  copy,
  save,
  saveAs,
  prepareDrag: prepareShared,
  dragFailed: (error) => showStatus(`No se pudo arrastrar la imagen: ${error}`, true),
  moreItems,
});

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
  redactPreferences.enabled = settings.autoRedact;
  redactPreferences.mode = settings.redactMode;
  editor.setColor(settings.annotationColor);

  refreshControls = bindInspector(sidebar, style, {
    changed: styleChanged,
    wallpaperPicked: requireWallpaper,
    edgeColor: () => (capture ? edgeColor(capture) : null),
  });
  bindPresets(sidebar.querySelector<HTMLElement>("#presets")!, settings.presets, {
    current: () => toStoredStyle(style),
    apply: (stored) => {
      applyStoredStyle(style, stored);
      refreshControls();
      styleChanged();
      requireWallpaper();
    },
    persist: (presets) => saveSetting("presets", presets),
    notify: (message) => showStatus(message),
  });
  showFound = bindRedactPanel(sidebar, redactPreferences, {
    enabledChanged: refreshAutoRedactions,
    categoryChanged: toggleRedaction,
    modeChanged: refreshAutoRedactions,
  });
  balanceToggle.checked = settings.balance;
  balanceToggle.addEventListener("change", () => {
    defaults.balance = balanceToggle.checked;
    saveSetting("balance", balanceToggle.checked);
    applyBalance(balanceToggle.checked);
  });
  syncToolbar = bindTopToolbar(titlebar, editor, {
    colorChanged: (color) => saveSetting("annotationColor", color),
    share: (anchor) => void share(anchor),
    notify: showStatus,
  });
  editor.setTool("select");

  // The Vite dev server is reachable from any browser, but only the Tauri window has the Rust
  // bridge. In development, `?preview=1` loads a sample capture to check the layout instead.
  if (outsideTauri) {
    if (import.meta.env.DEV && new URLSearchParams(location.search).has("preview")) {
      const { loadPreview } = await import("./dev-preview");
      await loadPreview((image) => showImage(image, 2, "preview"));
      return;
    }
    document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => (button.disabled = true));
    showStatus("Esto es la vista web en un navegador. Usa la ventana de Snapclone que abre «npm run tauri dev».", true);
  }
}

const outsideTauri = !isTauri();
init().catch((error) => showStatus(`No se pudo iniciar el editor: ${error}`, true));

document.querySelectorAll<HTMLButtonElement>("[data-action=capture]").forEach((button) => {
  button.addEventListener("click", startCapture);
});
canvas.addEventListener("dblclick", (event) => {
  if (capture && editor.tool === "select" && !editor.isOverCapture(event)) void copyAndClose();
});
canvasArea.addEventListener("dblclick", (event) => {
  if (capture && event.target === canvasArea.firstElementChild) void copyAndClose();
});

const historyView = new HistoryView({
  open: reopenCapture,
  deleted: captureDeleted,
  notify: showStatus,
});

const textView = new TextView();

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

document.addEventListener("keydown", (event) => {
  if (historyView.isOpen || textView.isOpen) {
    if (event.key === "Escape") {
      historyView.close();
      textView.close();
    }
    return;
  }
  if (settingsView.isOpen || popoverContains(event.target)) return;
  const editingText = event.target instanceof HTMLInputElement && event.target.type === "text";
  const inForm = editingText || event.target instanceof HTMLSelectElement;
  // ⇧⌘Z: native redo in a field, the editor's redo with a capture, otherwise reopen the last one.
  if (event.metaKey && event.shiftKey && !event.altKey && !event.ctrlKey && event.key.toLowerCase() === "z") {
    if (isEditableTarget(event.target)) return;
    event.preventDefault();
    if (capture) editor.handleKey(event);
    else if (!outsideTauri) reopenLast(showStatus);
    return;
  }
  if (capture && !inForm && editor.handleKey(event)) {
    event.preventDefault();
    return;
  }
  if (event.metaKey && event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
    event.preventDefault();
    saveAs();
    return;
  }
  if (!event.metaKey || event.shiftKey || event.altKey) {
    // ⌘+ is ⇧⌘= on most layouts.
    if (event.metaKey && !event.altKey && event.key === "+") {
      event.preventDefault();
      zoom.zoomIn();
    }
    return;
  }
  if (event.key === "=" || event.key === "+") {
    event.preventDefault();
    zoom.zoomIn();
  } else if (event.key === "-") {
    event.preventDefault();
    zoom.zoomOut();
  } else if (event.key === "0") {
    event.preventDefault();
    zoom.set("fit");
  } else if (event.repeat) {
    return;
  } else if (event.key === "c" && !editingText) {
    event.preventDefault();
    copy();
  } else if (event.key === "s") {
    event.preventDefault();
    save();
  } else if (event.key === "v" && !isEditableTarget(event.target) && !outsideTauri) {
    event.preventDefault();
    openClipboard(showStatus);
  }
});

if (!outsideTauri) {
  loadShortcuts()
    .then(showZoneShortcut)
    .catch((error) => console.error("shortcuts could not be read", error));
  listen<CaptureReady>("capture-ready", (event) => {
    historyView.close();
    textView.close();
    settingsView.close();
    loadCapture(event.payload).catch((error) => showStatus(`No se pudo cargar la captura: ${error}`, true));
  });
  listenNotices(showStatus);
  bindReopen((id) => {
    historyView.close();
    textView.close();
    settingsView.close();
    return reopenCapture(id);
  }, showStatus);
  highlightDrops(document.querySelector<HTMLElement>(".canvas-area")!);
  // "Ajustes…" in the app menu (⌘,) and in the menu bar icon.
  listen("open-settings", openSettings);
  listen<string>("capture-failed", (event) => {
    historyView.close();
    showStatus(`No se pudo hacer la captura: ${event.payload}`, true);
  });
}
