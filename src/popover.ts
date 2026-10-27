/**
 * Anchored popovers and pull-down menus in the style of macOS. One is open at a time; a click
 * outside, Escape or opening another closes it.
 */
export type Placement = "below" | "above" | "left";

export type MenuItem =
  | {
      label: string;
      action: () => void;
      shortcut?: string;
      disabled?: boolean;
      checked?: boolean;
    }
  | "separator";

const GAP = 6;
const EDGE = 8;

let current: { element: HTMLElement; anchor: HTMLElement; onClose?: () => void } | null = null;

export function closePopover(): void {
  const open = current;
  if (!open) return;
  current = null;
  open.element.remove();
  open.anchor.setAttribute("aria-expanded", "false");
  open.onClose?.();
}

export function isPopoverOpen(): boolean {
  return current !== null;
}

/** Whether the open popover belongs to `anchor`; clicking the anchor again then closes it. */
export function isOpenFor(anchor: HTMLElement): boolean {
  return current?.anchor === anchor;
}

/** Shows `content` next to `anchor`. Returns the popover element. */
export function openPopover(
  anchor: HTMLElement,
  content: HTMLElement,
  placement: Placement = "below",
  onClose?: () => void,
  className = "popover",
): HTMLElement {
  closePopover();
  const element = document.createElement("div");
  element.className = className;
  element.setAttribute("role", className === "menu" ? "menu" : "dialog");
  element.append(content);
  document.body.append(element);
  current = { element, anchor, onClose };
  anchor.setAttribute("aria-expanded", "true");
  position(element, anchor, placement);
  return element;
}

/** A menu of commands under (or above) `anchor`; toggles when the same anchor is clicked. */
export function showMenu(anchor: HTMLElement, items: MenuItem[], placement: Placement = "below"): void {
  if (isOpenFor(anchor)) {
    closePopover();
    return;
  }
  const list = document.createElement("div");
  list.className = "menu-items";
  for (const item of items) {
    if (item === "separator") {
      const separator = document.createElement("div");
      separator.className = "menu-separator";
      separator.setAttribute("role", "separator");
      list.append(separator);
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "menu-item";
    button.setAttribute("role", item.checked === undefined ? "menuitem" : "menuitemradio");
    if (item.checked !== undefined) button.setAttribute("aria-checked", String(item.checked));
    button.disabled = item.disabled ?? false;
    const check = document.createElement("span");
    check.className = "menu-check";
    check.textContent = item.checked ? "✓" : "";
    const label = document.createElement("span");
    label.className = "menu-label";
    label.textContent = item.label;
    const shortcut = document.createElement("span");
    shortcut.className = "menu-shortcut";
    shortcut.textContent = item.shortcut ?? "";
    button.append(check, label, shortcut);
    button.addEventListener("click", () => {
      closePopover();
      item.action();
    });
    list.append(button);
  }
  openPopover(anchor, list, placement, undefined, "menu");
}

function position(element: HTMLElement, anchor: HTMLElement, placement: Placement): void {
  const box = anchor.getBoundingClientRect();
  const { width, height } = element.getBoundingClientRect();
  const maxLeft = window.innerWidth - width - EDGE;
  const maxTop = window.innerHeight - height - EDGE;
  let left: number;
  let top: number;
  if (placement === "left") {
    left = box.left - width - GAP;
    top = box.top + box.height / 2 - height / 2;
  } else {
    left = box.left;
    top = placement === "below" ? box.bottom + GAP : box.top - height - GAP;
    // Flip when there is no room on the preferred side.
    if (placement === "below" && top > maxTop) top = box.top - height - GAP;
    if (placement === "above" && top < EDGE) top = box.bottom + GAP;
  }
  element.style.left = `${Math.round(Math.min(Math.max(left, EDGE), maxLeft))}px`;
  element.style.top = `${Math.round(Math.min(Math.max(top, EDGE), maxTop))}px`;
}

document.addEventListener(
  "pointerdown",
  (event) => {
    if (!current) return;
    const target = event.target as Node;
    if (current.element.contains(target) || current.anchor.contains(target)) return;
    closePopover();
  },
  true,
);

document.addEventListener(
  "keydown",
  (event) => {
    if (!current) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      const anchor = current.anchor;
      closePopover();
      anchor.focus();
    }
  },
  true,
);

/** Keys typed in a popover field must not trigger editor shortcuts. */
export function popoverContains(target: EventTarget | null): boolean {
  return current !== null && target instanceof Node && current.element.contains(target);
}

window.addEventListener("resize", closePopover);
