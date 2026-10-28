// A search snippet (docs/ARCHITECTURE.md §5 search, §9.6.6): parts with `hit` become <mark>, everything is rendered
// as text nodes, never as HTML. Text that still carries the FTS markers U+E000 (start) and U+E001 (end) is split on
// them too, so a server that sends the raw snippet renders the same.
import type { SnippetPart } from '../../api/gen/SnippetPart';

const START = '';
const END = '';

/** Splits marker-delimited text into parts; `inHit` is the state before the text. */
export function splitMarkers(text: string, inHit = false): SnippetPart[] {
  const out: SnippetPart[] = [];
  let hit = inHit;
  let buf = '';
  const flush = () => {
    if (buf) out.push({ text: buf, hit });
    buf = '';
  };
  for (const ch of text) {
    if (ch === START) {
      flush();
      hit = true;
    } else if (ch === END) {
      flush();
      hit = false;
    } else {
      buf += ch;
    }
  }
  flush();
  return out;
}

/** The snippet as display parts: markers resolved, neighbouring parts of the same kind merged. */
export function snippetParts(parts: readonly SnippetPart[] | string): SnippetPart[] {
  const raw = typeof parts === 'string' ? splitMarkers(parts) : parts.flatMap((p) => splitMarkers(p.text, p.hit));
  const out: SnippetPart[] = [];
  for (const p of raw) {
    const last = out[out.length - 1];
    if (last && last.hit === p.hit) last.text += p.text;
    else out.push({ ...p });
  }
  return out;
}

export function Snippet({ parts }: { parts: readonly SnippetPart[] | string }) {
  return (
    <p className="snippet">
      {snippetParts(parts).map((p, i) => (p.hit ? <mark key={i}>{p.text}</mark> : <span key={i}>{p.text}</span>))}
    </p>
  );
}
