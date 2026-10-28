// Minimal static file server for the Playwright `frame` project (docs/ARCHITECTURE.md §14): it serves the dist of
// scripts/inkboard.sh (web/dist-inkboard, §10.5) on 127.0.0.1, with `.wasm` as `application/wasm` so that
// `WebAssembly.instantiateStreaming` works. Owned by W5 after W0b.
//
//   node e2e/serve-static.mjs <root> <port>      (port 0 picks a free one; the "listening" line on stdout names it)
//
// playwright.config.ts starts it as its webServer for every project. A missing <root> is not an error, because the
// `smoke` project never builds the dist: the server warns once and answers 404 until the directory exists.
// Files are served as they are (no .br/.gz negotiation, no CSP); the app's own headers are quire-server's (§11).
import { createReadStream } from 'node:fs';
import { realpath, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, resolve, sep } from 'node:path';

const HOST = '127.0.0.1';

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
  res.writeHead(200, {
    'Content-Type': TYPES[extname(found.file).toLowerCase()] ?? 'application/octet-stream',
    'Content-Length': found.size,
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
  });
  if (req.method === 'HEAD') {
    res.end();
    return;
  }
  createReadStream(found.file)
    .on('error', (error) => {
      console.error(`serve-static: reading ${found.file}: ${error.message}`);
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
