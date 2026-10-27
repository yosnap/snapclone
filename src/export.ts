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

/** Returns false when the user cancels the save dialog. */
export async function saveToFile(canvas: HTMLCanvasElement): Promise<boolean> {
  const path = await invoke<string | null>("choose_save_path", { defaultName: defaultFileName() });
  if (!path) return false;

  const type = /\.jpe?g$/i.test(path) ? "image/jpeg" : "image/png";
  await invoke("save_image", await encode(canvas, type));
  return true;
}

function defaultFileName(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
  return `Snapclone ${date} a las ${time}.png`;
}
