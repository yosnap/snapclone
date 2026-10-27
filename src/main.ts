import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type CaptureReady = { id: string };

const canvas = document.querySelector<HTMLCanvasElement>("#editor-canvas")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const captureButton = document.querySelector<HTMLButtonElement>("#capture-button")!;

function captureUrl(id: string): string {
  return `capture://localhost/${id}`;
}

async function loadCapture(id: string): Promise<void> {
  const image = new Image();
  // Served with CORS so the canvas is not tainted and can be exported later.
  image.crossOrigin = "anonymous";
  image.src = captureUrl(id);
  await image.decode();

  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  canvas.getContext("2d")!.drawImage(image, 0, 0);
  emptyState.hidden = true;
  canvas.hidden = false;
}

captureButton.addEventListener("click", () => {
  invoke("start_capture").catch((error) => console.error("start_capture failed", error));
});

listen<CaptureReady>("capture-ready", (event) => {
  loadCapture(event.payload.id).catch((error) => console.error("capture load failed", error));
});
