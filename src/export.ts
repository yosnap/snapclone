import { invoke } from "@tauri-apps/api/core";

const JPEG_QUALITY = 0.9;
const JPEG_MATTE = "#ffffff";

function encode(canvas: HTMLCanvasElement, type: "image/png" | "image/jpeg"): Promise<Uint8Array> {
  const source = type === "image/jpeg" ? flattened(canvas) : canvas;
  return new Promise((resolve, reject) => {
    source.toBlob(
      (blob) => {
        if (!blob) {
          reject(new Error("the canvas could not be encoded"));
          return;
        }
        blob.arrayBuffer().then((buffer) => resolve(new Uint8Array(buffer)), reject);
      },
      type,
      type === "image/jpeg" ? JPEG_QUALITY : undefined,
    );
  });
}

/** JPEG has no alpha; without a matte, transparent pixels would turn black. */
function flattened(canvas: HTMLCanvasElement): HTMLCanvasElement {
  const copy = document.createElement("canvas");
  copy.width = canvas.width;
  copy.height = canvas.height;
  const ctx = copy.getContext("2d")!;
  ctx.fillStyle = JPEG_MATTE;
  ctx.fillRect(0, 0, copy.width, copy.height);
  ctx.drawImage(canvas, 0, 0);
  return copy;
}

export async function copyToClipboard(canvas: HTMLCanvasElement): Promise<void> {
  await invoke("copy_png", await encode(canvas, "image/png"));
}

/** Rust builds every file name from the user's pattern; the page only says when it is. */
function exportTime(): { headers: Record<string, string> } {
  return { headers: { "x-datetime": localDateTime() } };
}

/** "Guardar ⌘S": the PNG goes to the remembered folder. Returns the path, or null if cancelled. */
export async function quickSave(canvas: HTMLCanvasElement): Promise<string | null> {
  return invoke<string | null>("quick_save", await encode(canvas, "image/png"), exportTime());
}

/** "Guardar como…": returns false when the user cancels the save dialog. */
export async function saveToFile(canvas: HTMLCanvasElement): Promise<boolean> {
  const path = await invoke<string | null>("choose_save_path", { datetime: localDateTime() });
  if (!path) return false;

  const type = /\.jpe?g$/i.test(path) ? "image/jpeg" : "image/png";
  await invoke("save_image", await encode(canvas, type));
  return true;
}

/** Writes the PNG that "Compartir" and "Arrástrame" hand to macOS. */
export async function prepareExport(canvas: HTMLCanvasElement): Promise<void> {
  await invoke("prepare_export", await encode(canvas, "image/png"), exportTime());
}

/** "2026-10-28 a las 00.24.05", the `{datetime}` of file names, in local time. */
function localDateTime(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
  return `${date} a las ${time}`;
}
