// External images (docs/ARCHITECTURE.md §6.1): a pasted remote <img> keeps its src with attachmentId null and shows
// as a placeholder card; it is never fetched.
import { screen } from '@testing-library/react';
import { Editor } from '@tiptap/react';
import { describe, expect, it, vi } from 'vitest';
import { render } from '../test/render';
import { schemaExtensions } from './schema';
import { ExternalImageCard } from './views';

describe('external images', () => {
  it('a pasted remote <img> becomes an image without attachment, and a data: one is dropped', () => {
    const editor = new Editor({ extensions: schemaExtensions() });
    editor.commands.setContent('<p>x</p><img src="https://tracker.example/pixel.png" alt="Pixel"><img src="data:image/png;base64,AAAA">');
    const images = (editor.getJSON().content ?? []).filter((n) => n.type === 'image');
    expect(images).toHaveLength(1);
    expect(images[0].attrs).toMatchObject({ src: 'https://tracker.example/pixel.png', attachmentId: null, alt: 'Pixel' });
    // Our own attachment URL is recognized.
    editor.commands.setContent('<img src="/api/v1/attachments/0192f0a0-4444-7000-8000-000000000001">');
    expect(editor.getJSON().content?.[0].attrs?.attachmentId).toBe('0192f0a0-4444-7000-8000-000000000001');
    editor.destroy();
  });

  it('renders a placeholder with an "Open link", and no <img>', () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const { container } = render(<ExternalImageCard src="https://tracker.example/pixel.png" alt="Pixel" />);
    expect(screen.getByText('External image, not loaded')).toBeTruthy();
    expect(container.querySelector('img')).toBeNull();
    const open = screen.getByRole('link');
    expect(open.getAttribute('href')).toBe('https://tracker.example/pixel.png');
    expect(open.getAttribute('rel')).toContain('noopener');
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('offers no link for a src outside the allow-list', () => {
    render(<ExternalImageCard src="javascript:alert(1)" alt={null} />);
    expect(screen.queryByRole('link')).toBeNull();
  });
});
