import { invoke } from "@tauri-apps/api/core";
import type { AnnotationEditor, Tool } from "./annotation-editor";
import { ANNOTATION_COLORS } from "./annotations";
import { closePopover, isOpenFor, openPopover } from "./popover";
import type { Notify } from "./ui-helpers";

/** The pills' tools, as the editor names them. */
const PILL_TOOLS: Record<string, Tool> = { text: "text", arrow: "arrow", redact: "pixelate", shape: "rect" };

const TIPS = [
  "Tip: usa las teclas numéricas (1, 2, 3, 4)",
  "Pulsa «1» para añadir texto",
  "Pulsa «2» para añadir una flecha",
  "Pulsa «3» para redactar una zona",
  "Pulsa «4» para añadir una forma",
];

type ToolbarHandlers = {
  colorChanged: (color: string) => void;
  share: (anchor: HTMLElement) => void;
  notify: Notify;
};

/**
 * The title bar's tools: one pill per annotation tool, the colour pill with its palette,
 * the tip, "Enviar comentarios" and "Compartir". Returns a function that reflects the
 * editor's tool and colour on the pills.
 */
export function bindTopToolbar(root: HTMLElement, editor: AnnotationEditor, handlers: ToolbarHandlers): () => void {
  const pills = [...root.querySelectorAll<HTMLButtonElement>("[data-tool]")];
  const colorButton = root.querySelector<HTMLButtonElement>("#color-button")!;
  const colorChip = root.querySelector<HTMLElement>("#color-chip")!;
  const tipButton = root.querySelector<HTMLButtonElement>("#tip-button")!;

  const sync = () => {
    for (const pill of pills) {
      pill.setAttribute("aria-pressed", String(PILL_TOOLS[pill.dataset.tool!] === editor.tool));
    }
    colorChip.style.background = editor.color;
  };

  for (const pill of pills) {
    pill.addEventListener("click", () => {
      const tool = PILL_TOOLS[pill.dataset.tool!];
      // Clicking the active tool again goes back to selecting and moving.
      editor.setTool(editor.tool === tool ? "select" : tool);
      sync();
    });
  }

  colorButton.addEventListener("click", () => {
    if (isOpenFor(colorButton)) {
      closePopover();
      return;
    }
    openPopover(colorButton, palette(editor.color, (color) => {
      editor.setColor(color);
      handlers.colorChanged(color);
      sync();
    }));
  });

  let tip = 0;
  tipButton.addEventListener("click", () => {
    tip = (tip + 1) % TIPS.length;
    tipButton.textContent = TIPS[tip];
  });

  root.querySelector<HTMLButtonElement>("#feedback-button")!.addEventListener("click", () => {
    invoke("send_feedback").catch((error) => handlers.notify(`No se pudo abrir el correo: ${error}`, true));
  });
  const share = root.querySelector<HTMLButtonElement>("#share-button")!;
  share.addEventListener("click", () => handlers.share(share));

  sync();
  return sync;
}

/** Swatches of the annotation colours plus the system colour picker. */
function palette(selected: string, pick: (color: string) => void): HTMLElement {
  const grid = document.createElement("div");
  grid.className = "palette";
  for (const color of ANNOTATION_COLORS) {
    const swatch = document.createElement("button");
    swatch.type = "button";
    swatch.className = "palette-swatch";
    swatch.style.background = color;
    swatch.title = color;
    swatch.setAttribute("aria-pressed", String(color.toLowerCase() === selected.toLowerCase()));
    swatch.addEventListener("click", () => {
      pick(color);
      closePopover();
    });
    grid.append(swatch);
  }
  const custom = document.createElement("label");
  custom.className = "palette-custom";
  const input = document.createElement("input");
  input.type = "color";
  input.value = /^#[0-9a-f]{6}$/i.test(selected) ? selected : ANNOTATION_COLORS[0];
  input.addEventListener("input", () => pick(input.value));
  custom.append(input, "Otro color…");
  grid.append(custom);
  return grid;
}
