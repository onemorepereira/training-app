/**
 * Global keyboard routing. One rule for Esc: close the topmost overlay (the
 * native <dialog> does that itself), else leave fullscreen, else nothing.
 * Esc never shows or hides the navigation.
 *
 * Ctrl+Cmd+F is deliberately not handled: on macOS Tauri's default app menu
 * binds it to native fullscreen, and handling it here too would toggle twice.
 */
export type KeyAction = 'none' | 'exit-fullscreen' | 'toggle-fullscreen';

export interface KeyState {
  overlayOpen: boolean;
  fullscreen: boolean;
  typing: boolean;
}

export function routeKey(key: string, s: KeyState): KeyAction {
  if (key === 'Escape') {
    if (s.overlayOpen) return 'none';
    return s.fullscreen ? 'exit-fullscreen' : 'none';
  }
  if (key === 'F11' && !s.typing) return 'toggle-fullscreen';
  return 'none';
}

const NON_TEXT_INPUTS = new Set(['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file']);

/** True when keystrokes go to a text field (so global shortcuts must not fire). */
export function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (tag === 'INPUT') return !NON_TEXT_INPUTS.has(((el as HTMLInputElement).type || 'text').toLowerCase());
  return (el as HTMLElement).isContentEditable === true;
}

/** An overlay is open when any modal dialog or marked popover is showing. */
export function overlayOpen(doc: Pick<Document, 'querySelector'>): boolean {
  return !!doc.querySelector('dialog[open], [data-overlay="open"]');
}
