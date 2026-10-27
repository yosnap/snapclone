import type { Annotation, Point } from "./annotations";
import { REDACT_CATEGORIES, type RedactCategory } from "./auto-redact";
import { type StoredStyle, isHexColor, isRecord, parseStoredStyle } from "./persisted-style";

/**
 * Editing state of one capture, stored in the history so it can be reopened as it was left.
 * Annotations are in the coordinates of the capture as shown; `shift` records how far
 * balancing had moved the content, so they can be placed again even if balancing were to
 * give a different result on reopening.
 */
export type CaptureDocument = {
  version: 1;
  style: StoredStyle;
  balance: boolean;
  shift: Point;
  annotations: Annotation[];
};

const MAX_ANNOTATIONS = 1000;
const MAX_TEXT_LENGTH = 1000;
const MAX_COORDINATE = 1e6;
const CATEGORIES = new Set<string>(REDACT_CATEGORIES.map((category) => category.id));

/** Validates a document read back from the database; invalid annotations are skipped. */
export function parseCaptureDocument(value: unknown): CaptureDocument | null {
  if (!isRecord(value) || value.version !== 1) return null;
  const style = parseStoredStyle(value.style);
  if (!style) return null;
  const annotations = Array.isArray(value.annotations)
    ? value.annotations.slice(0, MAX_ANNOTATIONS).map(parseAnnotation).filter((item) => item !== null)
    : [];
  return {
    version: 1,
    style,
    balance: value.balance === true,
    shift: parsePoint(value.shift) ?? { x: 0, y: 0 },
    annotations,
  };
}

function parseAnnotation(value: unknown): Annotation | null {
  if (!isRecord(value) || typeof value.id !== "string" || !value.id) return null;
  const { id } = value;
  switch (value.kind) {
    case "arrow": {
      const from = parsePoint(value.from);
      const to = parsePoint(value.to);
      return from && to && isHexColor(value.color) ? { id, kind: "arrow", color: value.color, from, to } : null;
    }
    case "rect": {
      const box = parseBox(value);
      return box && isHexColor(value.color) ? { id, kind: "rect", color: value.color, ...box } : null;
    }
    case "text": {
      const at = parsePoint(value.at);
      if (!at || !isHexColor(value.color) || typeof value.text !== "string" || !value.text) return null;
      return { id, kind: "text", color: value.color, at, text: value.text.slice(0, MAX_TEXT_LENGTH) };
    }
    case "pixelate": {
      const box = parseBox(value);
      if (!box) return null;
      if (value.auto === undefined) return { id, kind: "pixelate", ...box };
      return typeof value.auto === "string" && CATEGORIES.has(value.auto)
        ? { id, kind: "pixelate", auto: value.auto as RedactCategory, ...box }
        : null;
    }
    default:
      return null;
  }
}

function isCoordinate(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && Math.abs(value) <= MAX_COORDINATE;
}

function parsePoint(value: unknown): Point | null {
  return isRecord(value) && isCoordinate(value.x) && isCoordinate(value.y) ? { x: value.x, y: value.y } : null;
}

function parseBox(value: Record<string, unknown>) {
  const { x, y, width, height } = value;
  if (!isCoordinate(x) || !isCoordinate(y) || !isCoordinate(width) || !isCoordinate(height)) return null;
  return width >= 0 && height >= 0 ? { x, y, width, height } : null;
}
