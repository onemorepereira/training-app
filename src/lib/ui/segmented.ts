/**
 * Keyboard model for a radio group: Arrow keys move and wrap, Home/End jump.
 * Returns the index to select, or null when the key doesn't move the selection.
 */
export function nextIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  const from = current < 0 ? 0 : current;
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return (from + 1) % count;
    case 'ArrowLeft':
    case 'ArrowUp':
      return (from - 1 + count) % count;
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    default:
      return null;
  }
}
