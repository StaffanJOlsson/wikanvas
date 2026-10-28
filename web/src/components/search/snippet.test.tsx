import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Snippet, snippetParts, splitMarkers } from './Snippet';

describe('search snippets', () => {
  it('an <img onerror> in a snippet renders as text, never as HTML', () => {
    const evil = '<img src=x onerror="window.__pwned = 1">';
    const { container } = render(<Snippet parts={[{ text: `before ${evil} after`, hit: false }, { text: '<b>bold</b>', hit: true }]} />);
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('b')).toBeNull();
    expect(container.textContent).toBe(`before ${evil} after<b>bold</b>`);
    expect(container.querySelector('mark')?.textContent).toBe('<b>bold</b>');
    expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined();
  });

  it('U+E000 / U+E001 become <mark>', () => {
    const { container } = render(<Snippet parts={[{ text: 'a quokka met a plan.', hit: false }]} />);
    expect([...container.querySelectorAll('mark')].map((m) => m.textContent)).toEqual(['quokka', 'plan']);
    expect(container.textContent).toBe('a quokka met a plan.');
    expect(container.textContent).not.toMatch(/[]/);
  });

  it('parts already marked as hits stay marks; neighbours of one kind merge', () => {
    expect(snippetParts([{ text: 'x ', hit: false }, { text: 'y', hit: true }, { text: 'z', hit: true }])).toEqual([
      { text: 'x ', hit: false },
      { text: 'yz', hit: true },
    ]);
    expect(splitMarkers('open')).toEqual([{ text: 'open', hit: true }]);
    expect(snippetParts('plain')).toEqual([{ text: 'plain', hit: false }]);
  });
});
