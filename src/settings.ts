import { type Store, load } from "@tauri-apps/plugin-store";
import {
  ARROW_HEAD_ORDER,
  ARROW_WIDTHS,
  REDACT_MODE_ORDER,
  SHAPE_ORDER,
  TEXT_SIZE_ORDER,
  type ToolOptions,
  defaultToolOptions,
} from "./annotation-editor";
import { ANNOTATION_COLORS } from "./annotations";
import { REDACT_CATEGORIES, REDACT_MODES, type RedactMode, type RedactSettings, defaultRedactSettings } from "./auto-redact";
import { type StoredStyle, isHexColor, isRecord, parseStoredStyle, toStoredStyle } from "./persisted-style";
import { defaultStyle } from "./style";

/** Lives in the app data directory, which the store plugin resolves relative paths against. */
const SETTINGS_FILE = "settings.json";
/** Slider drags fire many changes; the file is written once they settle. */
const SAVE_DELAY_MS = 300;
export const PRESET_NAME_MAX_LENGTH = 40;
export const PRESETS_MAX = 50;

export type Preset = { name: string; style: StoredStyle };

export type Settings = {
  style: StoredStyle;
  redact: RedactSettings;
  /** The main "Redactar datos sensibles" checkbox; the categories apply while it is on. */
  autoRedact: boolean;
  redactMode: RedactMode;
  annotationColor: string;
  toolOptions: ToolOptions;
  balance: boolean;
  presets: Preset[];
};

type Key = keyof Settings;

let store: Store | null = null;
const pending = new Map<Key, unknown>();
let saveTimer: number | undefined;

export function defaultSettings(): Settings {
  return {
    style: toStoredStyle(defaultStyle()),
    redact: defaultRedactSettings(),
    autoRedact: true,
    redactMode: "pixelate",
    annotationColor: ANNOTATION_COLORS[0],
    toolOptions: defaultToolOptions(),
    balance: false,
    presets: [],
  };
}

/**
 * Reads the saved settings, validating every field because the file can be edited by hand
 * or come from an older version. Without the store (a plain browser) the defaults are used
 * and nothing is written.
 */
export async function loadSettings(): Promise<Settings> {
  const settings = defaultSettings();
  try {
    store = await load(SETTINGS_FILE, { autoSave: false, defaults: {} });
  } catch (error) {
    console.error("settings store unavailable, using defaults", error);
    return settings;
  }
  const read = async (key: Key) => {
    try {
      return await store!.get<unknown>(key);
    } catch (error) {
      console.error(`setting ${key} could not be read`, error);
      return undefined;
    }
  };
  const keys = ["style", "redact", "annotationColor", "balance", "presets", "autoRedact", "redactMode", "toolOptions"] as const;
  const [style, redact, annotationColor, balance, presets, autoRedact, redactMode, toolOptions] = await Promise.all(
    keys.map(read),
  );
  settings.style = parseStoredStyle(style) ?? settings.style;
  settings.redact = parseRedact(redact) ?? settings.redact;
  if (isHexColor(annotationColor)) settings.annotationColor = annotationColor;
  if (typeof balance === "boolean") settings.balance = balance;
  if (typeof autoRedact === "boolean") settings.autoRedact = autoRedact;
  if (REDACT_MODES.includes(redactMode as RedactMode)) settings.redactMode = redactMode as RedactMode;
  settings.presets = parsePresets(presets);
  settings.toolOptions = parseToolOptions(toolOptions);
  return settings;
}

/** Queues a value and writes the file shortly after the last change. */
export function saveSetting<K extends Key>(key: K, value: Settings[K]): void {
  if (!store) return;
  pending.set(key, value);
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(flush, SAVE_DELAY_MS);
}

async function flush(): Promise<void> {
  const target = store;
  if (!target) return;
  const entries = [...pending];
  pending.clear();
  try {
    for (const [key, value] of entries) await target.set(key, value);
    await target.save();
  } catch (error) {
    console.error("settings could not be saved", error);
  }
}

function parseRedact(value: unknown): RedactSettings | null {
  if (!isRecord(value)) return null;
  const settings = defaultRedactSettings();
  for (const { id } of REDACT_CATEGORIES) {
    if (typeof value[id] === "boolean") settings[id] = value[id];
  }
  return settings;
}

function oneOf<T>(allowed: readonly T[], value: unknown, fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

function parseToolOptions(value: unknown): ToolOptions {
  const fallback = defaultToolOptions();
  if (!isRecord(value)) return fallback;
  return {
    textSize: oneOf(TEXT_SIZE_ORDER, value.textSize, fallback.textSize),
    arrowHead: oneOf(ARROW_HEAD_ORDER, value.arrowHead, fallback.arrowHead),
    arrowWidth: oneOf<number>(ARROW_WIDTHS, value.arrowWidth, fallback.arrowWidth),
    redactMode: oneOf(REDACT_MODE_ORDER, value.redactMode, fallback.redactMode),
    shape: oneOf(SHAPE_ORDER, value.shape, fallback.shape),
  };
}

export function normalizePresetName(name: string): string {
  return name.trim().slice(0, PRESET_NAME_MAX_LENGTH);
}

function parsePresets(value: unknown): Preset[] {
  if (!Array.isArray(value)) return [];
  const presets: Preset[] = [];
  for (const item of value.slice(0, PRESETS_MAX)) {
    if (!isRecord(item) || typeof item.name !== "string") continue;
    const name = normalizePresetName(item.name);
    const style = parseStoredStyle(item.style);
    if (name && style && !presets.some((preset) => preset.name === name)) presets.push({ name, style });
  }
  return presets;
}
