import { describe, it, expect } from 'vitest';
import { routeKey, isTypingTarget, overlayOpen } from './keys';

const base = { overlayOpen: false, fullscreen: false, typing: false };

describe('routeKey', () => {
  it('Esc with a dialog open does nothing (the dialog closes itself)', () => {
    expect(routeKey('Escape', { ...base, overlayOpen: true, fullscreen: true })).toBe('none');
  });

  it('Esc in fullscreen with no dialog exits fullscreen', () => {
    expect(routeKey('Escape', { ...base, fullscreen: true })).toBe('exit-fullscreen');
  });

  it('Esc with nothing open does nothing', () => {
    expect(routeKey('Escape', base)).toBe('none');
  });

  it('F11 toggles fullscreen', () => {
    expect(routeKey('F11', base)).toBe('toggle-fullscreen');
  });

  it('F11 while typing in a field does nothing', () => {
    expect(routeKey('F11', { ...base, typing: true })).toBe('none');
  });

  it('other keys do nothing', () => {
    expect(routeKey('f', { ...base, fullscreen: true })).toBe('none');
  });
});

const el = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, ...extra }) as unknown as Element;

describe('isTypingTarget', () => {
  it('text inputs, textareas and selects are typing targets', () => {
    expect(isTypingTarget(el('INPUT', { type: 'text' }))).toBe(true);
    expect(isTypingTarget(el('INPUT', { type: 'number' }))).toBe(true);
    expect(isTypingTarget(el('TEXTAREA'))).toBe(true);
    expect(isTypingTarget(el('SELECT'))).toBe(true);
  });

  it('checkboxes, buttons and plain elements are not', () => {
    expect(isTypingTarget(el('INPUT', { type: 'checkbox' }))).toBe(false);
    expect(isTypingTarget(el('BUTTON'))).toBe(false);
    expect(isTypingTarget(el('DIV', { isContentEditable: false }))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });

  it('contenteditable elements are typing targets', () => {
    expect(isTypingTarget(el('DIV', { isContentEditable: true }))).toBe(true);
  });
});

describe('overlayOpen', () => {
  it('asks the document for an open dialog or marked overlay', () => {
    let asked = '';
    const doc = { querySelector: (s: string) => { asked = s; return null; } };
    expect(overlayOpen(doc)).toBe(false);
    expect(asked).toBe('dialog[open], [data-overlay="open"]');
    expect(overlayOpen({ querySelector: () => ({}) as Element })).toBe(true);
  });
});
