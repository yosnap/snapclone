/** Shows a message in a status line; errors are styled as such. */
export type Notify = (message: string, isError?: boolean) => void;

/**
 * Whether a key event goes to a field the user types or picks in. Editor shortcuts that
 * clash with editing (⇧⌘Z, ⌘V) leave such targets to their native behaviour.
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}
