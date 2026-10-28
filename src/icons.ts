/**
 * Inline SVG icons drawn for Snapclone in a thin-stroke style close to SF Symbols. They are
 * original drawings, not copies of Xnapper's assets. `[data-icon=name]` placeholders in the
 * HTML are filled once at startup; `icon()` builds one for elements created in code.
 */
const STROKE = 'fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"';

const PATHS: Record<string, string> = {
  text: `<path ${STROKE} d="M3.5 18 8.5 5.5h1L14.5 18M5.4 13.5h7.2M18.5 4.5v15M16.5 4.5h4M16.5 19.5h4"/>`,
  arrow: `<path ${STROKE} d="M18.5 18.5 6 6M6 14V6h8"/>`,
  eraser: `<path ${STROKE} d="m14.2 4.6 5.2 5.2a1.6 1.6 0 0 1 0 2.3L12 19.5H7.8l-3.2-3.2a1.6 1.6 0 0 1 0-2.3l9.6-9.4M9 9.8l5.2 5.2M12 19.5h8"/>`,
  shapes: `<rect ${STROKE} x="3.5" y="3.5" width="11" height="11" rx="1.5"/><circle ${STROKE} cx="14.5" cy="14.5" r="5.5"/>`,
  share: `<path ${STROKE} d="M12 14.5V3.5M8.5 7 12 3.5 15.5 7M8.5 10.5H7a1.5 1.5 0 0 0-1.5 1.5v7A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5v-7a1.5 1.5 0 0 0-1.5-1.5h-1.5"/>`,
  grip: `<g fill="currentColor"><circle cx="8.5" cy="5.5" r="1.9"/><circle cx="15.5" cy="5.5" r="1.9"/><circle cx="8.5" cy="12" r="1.9"/><circle cx="15.5" cy="12" r="1.9"/><circle cx="8.5" cy="18.5" r="1.9"/><circle cx="15.5" cy="18.5" r="1.9"/></g>`,
  "chevron-down": `<path ${STROKE} stroke-width="2.4" d="m5.5 9 6.5 6.5L18.5 9"/>`,
  "chevron-up-down": `<path ${STROKE} stroke-width="2" d="m8 9.5 4-4 4 4M8 14.5l4 4 4-4"/>`,
  trash: `<path ${STROKE} d="M4.5 6.5h15M9.5 6.5V4.8c0-.7.5-1.3 1.2-1.3h2.6c.7 0 1.2.6 1.2 1.3v1.7M6.5 6.5l.9 12.1c.1.9.8 1.4 1.6 1.4h6c.8 0 1.5-.5 1.6-1.4l.9-12.1M10 10.5v6M14 10.5v6"/>`,
  gear: `<path fill="currentColor" fill-rule="evenodd" d="M10.3 2.5h3.4l.5 2.6c.6.2 1.1.5 1.6.9l2.5-.9 1.7 2.9-2 1.7c.1.6.1 1.2 0 1.8l2 1.7-1.7 2.9-2.5-.9c-.5.4-1 .7-1.6.9l-.5 2.6h-3.4l-.5-2.6c-.6-.2-1.1-.5-1.6-.9l-2.5.9-1.7-2.9 2-1.7a6 6 0 0 1 0-1.8l-2-1.7 1.7-2.9 2.5.9c.5-.4 1-.7 1.6-.9zM12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4z"/>`,
  check: `<path ${STROKE} stroke-width="2.2" d="m5.5 12.5 4 4 9-9"/>`,
  rect: `<rect ${STROKE} x="4" y="6" width="16" height="12" rx="1"/>`,
  oval: `<ellipse ${STROKE} cx="12" cy="12" rx="8" ry="6"/>`,
  line: `<path ${STROKE} d="M5 19 19 5"/>`,
  "arrow-filled": `<path ${STROKE} d="M5 19 15 9"/><path fill="currentColor" d="M19.5 4.5 17.6 13 11 6.4z"/>`,
  "arrow-open": `<path ${STROKE} d="M5 19 19 5M11 5h8v8"/>`,
  "arrow-double": `<path ${STROKE} d="M6.5 17.5 17.5 6.5M5 12v7h7M12 5h7v7"/>`,
  pixelate: `<g fill="currentColor"><rect x="4" y="4" width="5" height="5" opacity=".9"/><rect x="9.5" y="4" width="5" height="5" opacity=".4"/><rect x="15" y="4" width="5" height="5" opacity=".7"/><rect x="4" y="9.5" width="5" height="5" opacity=".5"/><rect x="9.5" y="9.5" width="5" height="5" opacity=".9"/><rect x="15" y="9.5" width="5" height="5" opacity=".3"/><rect x="4" y="15" width="5" height="5" opacity=".7"/><rect x="9.5" y="15" width="5" height="5" opacity=".3"/><rect x="15" y="15" width="5" height="5" opacity=".8"/></g>`,
  solid: `<rect fill="currentColor" x="4" y="7" width="16" height="10" rx="1"/>`,
};

export function icon(name: string, size = 16): SVGSVGElement {
  const template = document.createElement("template");
  template.innerHTML = `<svg class="icon" width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true">${PATHS[name] ?? ""}</svg>`;
  return template.content.firstElementChild as SVGSVGElement;
}

/** Replaces every `[data-icon]` placeholder under `root`; `data-icon-size` sets the size. */
export function fillIcons(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-icon]").forEach((placeholder) => {
    const size = Number(placeholder.dataset.iconSize) || 16;
    placeholder.replaceWith(icon(placeholder.dataset.icon!, size));
  });
}
