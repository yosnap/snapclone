import { invoke } from "@tauri-apps/api/core";
import type { PixelateAnnotation } from "./annotations";

export type RedactCategory = "email" | "credit-card" | "ip-address" | "api-key";

/** Normalized (0..1) box from the backend, origin at the capture's top-left corner. */
export type SensitiveRegion = { category: RedactCategory; x: number; y: number; width: number; height: number };

export const REDACT_CATEGORIES: { id: RedactCategory; label: string }[] = [
  { id: "email", label: "Direcciones de correo" },
  { id: "credit-card", label: "Tarjetas de crédito" },
  { id: "ip-address", label: "Direcciones IP" },
  { id: "api-key", label: "Posibles claves de API y contraseñas" },
];

/** How a redacted region is hidden: pixelated, or covered with a solid block. */
export type RedactMode = "pixelate" | "solid";
export const REDACT_MODES: RedactMode[] = ["pixelate", "solid"];

/** Breathing room around Vision's tight glyph boxes, in points. */
const REGION_MARGIN = 2;

export type RedactSettings = Record<RedactCategory, boolean>;

export function defaultRedactSettings(): RedactSettings {
  return { email: true, "credit-card": true, "ip-address": true, "api-key": true };
}

export function detectSensitive(captureId: string): Promise<SensitiveRegion[]> {
  return invoke<SensitiveRegion[]>("detect_sensitive", { id: captureId });
}

export function toPixelateAnnotations(
  regions: SensitiveRegion[],
  imageWidth: number,
  imageHeight: number,
  scale: number,
): PixelateAnnotation[] {
  const margin = REGION_MARGIN * scale;
  return regions.map((region) => {
    const x = Math.max(0, region.x * imageWidth - margin);
    const y = Math.max(0, region.y * imageHeight - margin);
    // Ids derive from the box so a capture reopened from the history, whose regions were
    // saved with it, gets the same ids when detection runs again and nothing is duplicated.
    const box = [region.x, region.y, region.width, region.height].map((value) => value.toFixed(6)).join(",");
    return {
      id: `auto:${region.category}:${box}`,
      kind: "pixelate",
      auto: region.category,
      x,
      y,
      width: Math.min(imageWidth, region.x * imageWidth + region.width * imageWidth + margin) - x,
      height: Math.min(imageHeight, region.y * imageHeight + region.height * imageHeight + margin) - y,
    };
  });
}
