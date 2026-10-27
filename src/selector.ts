import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

/**
 * One overlay per monitor, showing that monitor frozen and dimmed. Dragging picks a zone;
 * Space switches every overlay to window mode, where a click captures the highlighted
 * window; Esc cancels. Coordinates are CSS pixels, which equal points in these windows.
 */
type WindowRect = { id: number; x: number; y: number; width: number; height: number };
type OverlayInfo = { pixelWidth: number; pixelHeight: number; windows: WindowRect[] };
type Point = { x: number; y: number };
type Rect = { x: number; y: number; width: number; height: number };

/** Shared by every overlay so window mode is the same on all monitors. */
const MODE_EVENT = "selector-mode";
const MIN_SIZE = 4;
const DIM = "rgba(0, 0, 0, 0.45)";
const OUTLINE = "#ffffff";
const LABEL_FONT = '12px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';

const canvas = document.querySelector<HTMLCanvasElement>("#selector-canvas")!;
const hint = document.querySelector<HTMLElement>("#selector-hint")!;
const ctx = canvas.getContext("2d")!;

let frame: ImageBitmap | null = null;
let windows: WindowRect[] = [];
let windowMode = false;
let pointer: Point | null = null;
let dragStart: Point | null = null;
/** Set once a result was sent, so nothing is sent twice while the overlays close. */
let finished = false;
let drawQueued = false;

async function init(): Promise<void> {
  const [info, buffer] = await Promise.all([
    invoke<OverlayInfo>("selector_info"),
    invoke<ArrayBuffer>("selector_frame"),
  ]);
  const pixels = new ImageData(new Uint8ClampedArray(buffer), info.pixelWidth, info.pixelHeight);
  frame = await createImageBitmap(pixels);
  windows = info.windows;
  canvas.width = info.pixelWidth;
  canvas.height = info.pixelHeight;
  await listen<boolean>(MODE_EVENT, (event) => {
    windowMode = event.payload === true;
    dragStart = null;
    hint.textContent = windowMode
      ? "Haz clic en una ventana · Espacio: zona · Esc: cancelar"
      : "Arrastra para elegir una zona · Espacio: ventana · Esc: cancelar";
    scheduleDraw();
  });
  scheduleDraw();
}

function scheduleDraw(): void {
  if (drawQueued) return;
  drawQueued = true;
  requestAnimationFrame(() => {
    drawQueued = false;
    draw();
  });
}

function draw(): void {
  if (!frame) return;
  // Draw in points; the canvas backing store is at the monitor's pixel resolution.
  const scale = canvas.width / window.innerWidth;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  const width = window.innerWidth;
  const height = window.innerHeight;
  ctx.drawImage(frame, 0, 0, width, height);
  ctx.fillStyle = DIM;
  ctx.fillRect(0, 0, width, height);

  const highlight = windowMode ? hoveredWindow() : dragRect();
  if (!highlight) return;
  const { x, y, width: w, height: h } = highlight;
  // Undimmed copy of the frozen image inside the highlighted area.
  ctx.drawImage(frame, x * scale, y * scale, w * scale, h * scale, x, y, w, h);
  ctx.lineWidth = 1;
  ctx.strokeStyle = OUTLINE;
  ctx.strokeRect(x + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
  drawSizeLabel(highlight, scale);
}

function drawSizeLabel(rect: Rect, scale: number): void {
  const text = `${Math.round(rect.width * scale)} × ${Math.round(rect.height * scale)}`;
  ctx.font = LABEL_FONT;
  const padding = 4;
  const labelWidth = ctx.measureText(text).width + padding * 2;
  const labelHeight = 18;
  const x = Math.min(rect.x, window.innerWidth - labelWidth);
  const y = rect.y >= labelHeight + 4 ? rect.y - labelHeight - 4 : rect.y + rect.height + 4;
  ctx.fillStyle = "rgba(0, 0, 0, 0.75)";
  ctx.fillRect(x, y, labelWidth, labelHeight);
  ctx.fillStyle = OUTLINE;
  ctx.textBaseline = "middle";
  ctx.fillText(text, x + padding, y + labelHeight / 2);
}

function dragRect(): Rect | null {
  if (!dragStart || !pointer) return null;
  return {
    x: Math.min(dragStart.x, pointer.x),
    y: Math.min(dragStart.y, pointer.y),
    width: Math.abs(pointer.x - dragStart.x),
    height: Math.abs(pointer.y - dragStart.y),
  };
}

/** Front-most window under the pointer; the list comes front to back. */
function hoveredWindow(): WindowRect | null {
  if (!pointer) return null;
  const { x, y } = pointer;
  return windows.find((item) => x >= item.x && y >= item.y && x < item.x + item.width && y < item.y + item.height) ?? null;
}

function finish(command: string, args: Record<string, unknown>): void {
  if (finished) return;
  finished = true;
  invoke(command, args).catch((error) => {
    // The overlays are already gone at this point; the log is all that is left.
    console.error(`${command} failed`, error);
  });
}

canvas.addEventListener("pointerdown", (event) => {
  if (event.button !== 0 || !frame) return;
  pointer = { x: event.clientX, y: event.clientY };
  if (windowMode) {
    const target = hoveredWindow();
    if (target) finish("finish_window", { id: target.id });
    return;
  }
  canvas.setPointerCapture(event.pointerId);
  dragStart = pointer;
  scheduleDraw();
});

canvas.addEventListener("pointermove", (event) => {
  pointer = { x: event.clientX, y: event.clientY };
  scheduleDraw();
});

canvas.addEventListener("pointerup", (event) => {
  if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  const rect = dragRect();
  dragStart = null;
  if (rect && rect.width >= MIN_SIZE && rect.height >= MIN_SIZE) finish("finish_selection", { rect });
  else scheduleDraw();
});

canvas.addEventListener("pointercancel", () => {
  dragStart = null;
  scheduleDraw();
});

document.addEventListener("contextmenu", (event) => event.preventDefault());

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    finish("cancel_selection", {});
  } else if (event.key === " " && !event.repeat) {
    event.preventDefault();
    emit(MODE_EVENT, !windowMode).catch((error) => console.error("window mode could not be shared", error));
  }
});

window.addEventListener("resize", scheduleDraw);

init().catch((error) => {
  console.error("selector could not start", error);
  // Without its frozen image the overlay is useless; end the selection rather than trap the user.
  finish("cancel_selection", {});
});
