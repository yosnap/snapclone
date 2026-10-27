import { invoke } from "@tauri-apps/api/core";
import type { AnnotationEditor, Tool } from "./annotation-editor";
import { ANNOTATION_COLORS } from "./annotations";
import { closePopover, isOpenFor, openPopover } from "./popover";
import { toolOptionsPanel } from "./tool-options";
import type { Notify } from "./ui-helpers";

const DEFAULT_TIP = "Tip: usa las teclas numéricas (1, 2, 3, 4)";
/** What the tip says while each tool is active. */
const TOOL_TIPS: Record<Tool, string> = {
  select: DEFAULT_TIP,
  text: "Haz clic para escribir · pulsa «1» otra vez para cambiar el tamaño",
  arrow: "Arrastra para dibujar una flecha · pulsa «2» otra vez para cambiar el estilo",
  redact: "Arrastra sobre lo que quieras tapar · pulsa «3» otra vez para cambiar el modo",
  shape: "Arrastra para dibujar · pulsa «4» otra vez para cambiar la forma",
};

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
      pill.setAttribute("aria-pressed", String(pill.dataset.tool === editor.tool));
    }
    colorChip.style.background = editor.color;
    tipButton.textContent = TOOL_TIPS[editor.tool];
  };

  for (const pill of pills) {
    pill.addEventListener("click", () => {
      const tool = pill.dataset.tool as Tool;
      // A click picks the tool; a second click shows its options.
      if (editor.tool !== tool) {
        closePopover();
        editor.setTool(tool);
      } else if (isOpenFor(pill)) {
        closePopover();
      } else {
        openPopover(pill, toolOptionsPanel(tool, editor));
      }
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

  // Clicking the tip goes back to selecting and moving annotations.
  tipButton.addEventListener("click", () => {
    editor.setTool("select");
    sync();
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
