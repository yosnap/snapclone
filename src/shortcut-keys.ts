/** Keys that only modify another; a shortcut is complete once a key outside these is pressed. */
const MODIFIER_CODES = new Set([
  "ShiftLeft",
  "ShiftRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "CapsLock",
  "Fn",
]);

/**
 * The shortcut a key press describes, in the `Ctrl+Alt+Shift+Cmd+<code>` form Rust parses,
 * or `null` while only modifiers are held. Physical key codes are used so the shortcut is
 * the same whatever the keyboard layout types.
 */
export function shortcutFromEvent(event: KeyboardEvent): string | null {
  if (!event.code || MODIFIER_CODES.has(event.code)) return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Cmd");
  parts.push(event.code);
  return parts.join("+");
}
