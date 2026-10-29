// Minimal static file server for the Playwright `frame` project (docs/ARCHITECTURE.md §14): it serves the dist of
// scripts/inkboard.sh (web/dist-inkboard, §10.5) on 127.0.0.1, with `.wasm` as `application/wasm` so that
// `WebAssembly.instantiateStreaming` works. Owned by W5.
//
//   node e2e/serve-static.mjs <root> <port>      (port 0 picks a free one; the "listening" line on stdout names it)
//
// playwright.config.ts starts it as its webServer for every project. A missing <root> is not an error, because the
// `smoke` project never builds the dist: the server warns once and answers 404 until the directory exists.
// What it shares with wikanvas-server's static serving (§11), so that the frame test runs the engine as production
// does: HTML gets the frame CSP and X-Frame-Options: SAMEORIGIN, and a file with precompressed variants is answered
// with `name.br` or `name.gz` (Content-Encoding, Vary: Accept-Encoding) when Accept-Encoding offers them. Caching is
// not shared: every answer is no-store.
import { createReadStream } from 'node:fs';
import { realpath, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, resolve, sep } from 'node:path';

const HOST = '127.0.0.1';

// The frame CSP of docs/ARCHITECTURE.md §11, sent with every HTML file.
const FRAME_CSP = [
  "default-src 'none'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self' data:",
  "connect-src 'self'",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'self'",
  "object-src 'none'",
].join('; ');

// Precompressed variants, in the order wikanvas-server prefers them (§11): brotli, then gzip.
const ENCODINGS = [
  ['br', '.br'],
  ['gzip', '.gz'],
];

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.txt': 'text/plain; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8',
  '.inkboard': 'application/json',
};

function usage(message) {
  console.error(`serve-static: ${message}\nusage: node e2e/serve-static.mjs <root> <port>`);
  process.exit(2);
}

const [rootArg, portArg] = process.argv.slice(2);
if (!rootArg || portArg === undefined || process.argv.length > 4) usage('expected exactly two arguments');
if (!/^\d+$/.test(portArg) || Number(portArg) > 65535) usage(`invalid port: ${portArg}`);
const root = resolve(rootArg);
const port = Number(portArg);

let warnedMissingRoot = false;

// The real path of the file a request names, or null (404). The check runs on real paths, so neither `..` nor a
// symlink leads outside the root.
async function locate(pathname) {
  let realRoot;
  try {
    realRoot = await realpath(root);
  } catch {
    if (!warnedMissingRoot) {
      console.error(`serve-static: ${root} does not exist (yet); answering 404`);
      warnedMissingRoot = true;
    }
    return null;
  }
  let name;
  try {
    name = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (name.includes('\0')) return null;
  if (name.endsWith('/')) name += 'index.html';
  try {
    const file = await realpath(resolve(realRoot, `.${name}`));
    if (!file.startsWith(realRoot + sep)) return null;
    const info = await stat(file);
    return info.isFile() ? { file, size: info.size } : null;
  } catch {
    return null;
  }
}

// The variant of `found` to send for this Accept-Encoding: { file, size, encoding } (encoding null: identity).
async function variant(found, acceptEncoding) {
  const offered = new Set(
    String(acceptEncoding ?? '')
      .split(',')
      .map((part) => part.split(';')[0].trim().toLowerCase())
      .filter(Boolean),
  );
  for (const [encoding, suffix] of ENCODINGS) {
    if (!offered.has(encoding)) continue;
    try {
      const info = await stat(found.file + suffix);
      if (info.isFile()) return { file: found.file + suffix, size: info.size, encoding };
    } catch {
      // no such variant
    }
  }
  return { ...found, encoding: null };
}

function send(res, status, text, headers = {}) {
  res.writeHead(status, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Content-Length': Buffer.byteLength(text),
    'Cache-Control': 'no-store',
    ...headers,
  });
  res.end(text);
}

const server = createServer(async (req, res) => {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    send(res, 405, 'method not allowed\n', { Allow: 'GET, HEAD' });
    return;
  }
  let pathname;
  try {
    pathname = new URL(req.url ?? '/', 'http://serve-static.invalid').pathname;
  } catch {
    send(res, 400, 'bad request\n');
    return;
  }
  const found = await locate(pathname);
  if (!found) {
    send(res, 404, 'not found\n');
    return;
  }
  const type = TYPES[extname(found.file).toLowerCase()] ?? 'application/octet-stream';
  const chosen = await variant(found, req.headers['accept-encoding']);
  const headers = {
    'Content-Type': type,
    'Content-Length': chosen.size,
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
    Vary: 'Accept-Encoding',
  };
  if (chosen.encoding) headers['Content-Encoding'] = chosen.encoding;
  if (type.startsWith('text/html')) {
    headers['Content-Security-Policy'] = FRAME_CSP;
    headers['X-Frame-Options'] = 'SAMEORIGIN';
  }
  res.writeHead(200, headers);
  if (req.method === 'HEAD') {
    res.end();
    return;
  }
  createReadStream(chosen.file)
    .on('error', (error) => {
      console.error(`serve-static: reading ${chosen.file}: ${error.message}`);
      res.destroy(error);
    })
    .pipe(res);
});

server.on('error', (error) => {
  console.error(`serve-static: ${error.message}`);
  process.exit(1);
});

server.listen(port, HOST, () => {
  const { port: bound } = server.address();
  console.log(`serve-static: serving ${root} on http://${HOST}:${bound}/`);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    server.close();
    server.closeAllConnections();
    process.exit(0);
  });
}
