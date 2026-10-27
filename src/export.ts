import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";

const JPEG_QUALITY = 0.9;

function encode(canvas: HTMLCanvasElement, type: "image/png" | "image/jpeg"): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
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

export async function copyToClipboard(canvas: HTMLCanvasElement): Promise<void> {
  await invoke("copy_png", await encode(canvas, "image/png"));
}

/** Returns false when the user cancels the save dialog. */
export async function saveToFile(canvas: HTMLCanvasElement): Promise<boolean> {
  const path = await save({
    defaultPath: defaultFileName(),
    filters: [
      { name: "PNG", extensions: ["png"] },
      { name: "JPEG", extensions: ["jpg", "jpeg"] },
    ],
  });
  if (!path) return false;

  const type = /\.jpe?g$/i.test(path) ? "image/jpeg" : "image/png";
  await invoke("save_image", await encode(canvas, type), {
    headers: { path: encodeURIComponent(path) },
  });
  return true;
}

function defaultFileName(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
  return `Snapclone ${date} a las ${time}.png`;
}
