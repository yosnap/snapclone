import {
  ARROW_HEAD_ORDER,
  ARROW_WIDTHS,
  type AnnotationEditor,
  REDACT_MODE_ORDER,
  SHAPE_ORDER,
  TEXT_SIZE_ORDER,
  type Tool,
  type ToolOptions,
} from "./annotation-editor";
import { icon } from "./icons";

type Choice<T> = { value: T; label: string; icon?: string };

const TEXT_LABELS = { small: "Pequeño", medium: "Mediano", large: "Grande" } as const;
const HEAD_CHOICES: Record<(typeof ARROW_HEAD_ORDER)[number], Choice<string>> = {
  filled: { value: "filled", label: "Punta llena", icon: "arrow-filled" },
  open: { value: "open", label: "Punta abierta", icon: "arrow-open" },
  double: { value: "double", label: "Doble punta", icon: "arrow-double" },
};
const WIDTH_LABELS = ["Fino", "Medio", "Grueso"];
const REDACT_CHOICES = { pixelate: { label: "Pixelado", icon: "pixelate" }, solid: { label: "Bloque sólido", icon: "solid" } };
const SHAPE_CHOICES = { rect: { label: "Rectángulo", icon: "rect" }, oval: { label: "Óvalo", icon: "oval" }, line: { label: "Línea", icon: "line" } };

/** The options popover of a toolbar pill: one segmented row per option of the tool. */
export function toolOptionsPanel(tool: Tool, editor: AnnotationEditor): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "tool-options";
  const add = <K extends keyof ToolOptions>(title: string, key: K, choices: Choice<ToolOptions[K]>[]) => {
    panel.append(segmented(title, choices, () => editor.options[key], (value) => editor.setOptions({ [key]: value } as Partial<ToolOptions>)));
  };
  switch (tool) {
    case "text":
      add("Tamaño", "textSize", TEXT_SIZE_ORDER.map((value) => ({ value, label: TEXT_LABELS[value] })));
      break;
    case "arrow":
      add("Estilo", "arrowHead", ARROW_HEAD_ORDER.map((value) => ({ ...HEAD_CHOICES[value], value })));
      add("Grosor", "arrowWidth", ARROW_WIDTHS.map((value, index) => ({ value, label: WIDTH_LABELS[index] })));
      break;
    case "redact":
      add("Modo", "redactMode", REDACT_MODE_ORDER.map((value) => ({ value, ...REDACT_CHOICES[value] })));
      break;
    case "shape":
      add("Forma", "shape", SHAPE_ORDER.map((value) => ({ value, ...SHAPE_CHOICES[value] })));
      break;
    case "select":
      break;
  }
  return panel;
}

function segmented<T>(title: string, choices: Choice<T>[], current: () => T, select: (value: T) => void): HTMLElement {
  const group = document.createElement("div");
  group.className = "option-group";
  const caption = document.createElement("p");
  caption.className = "popover-caption";
  caption.textContent = title;
  const row = document.createElement("div");
  row.className = "segmented";
  row.setAttribute("role", "radiogroup");
  row.setAttribute("aria-label", title);
  const buttons = choices.map((choice) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "segment";
    button.setAttribute("role", "radio");
    if (choice.icon) button.append(icon(choice.icon, 16));
    button.append(choice.label);
    button.addEventListener("click", () => {
      select(choice.value);
      sync();
    });
    return { button, choice };
  });
  const sync = () => {
    for (const { button, choice } of buttons) button.setAttribute("aria-checked", String(choice.value === current()));
  };
  sync();
  row.append(...buttons.map(({ button }) => button));
  group.append(caption, row);
  return group;
}
