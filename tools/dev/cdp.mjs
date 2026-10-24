// Evaluate JavaScript in a Pill WebView2 page over the Chrome DevTools Protocol (local debug port only).
//
//   node tools/dev/cdp.mjs list                              list the pages the app exposes
//   node tools/dev/cdp.mjs pill.html "Pill.state"            evaluate an expression (promises are awaited)
//   node tools/dev/cdp.mjs pill.html "document.title" --port 9333 --timeout 8000
//
// The first argument is a substring of the page URL ("pill.html", "bar.html", "settings.html"). The JSON result is printed.
// Start the app first with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333 --remote-allow-origins=*"
// (see tools/dev/README.md). Needs Node 22+ (global WebSocket). Also imported by prof.mjs and audit-transitions.mjs.
import { parseArgs } from 'node:util';
import { pathToFileURL } from 'node:url';

export const DEFAULT_PORT = Number(process.env.CDP_PORT) || 9333;

export async function listTargets(port = DEFAULT_PORT) {
  try {
    return await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  } catch (e) {
    throw new Error(`cannot reach the debug port ${port} (${e.message}). Is the app running with --remote-debugging-port=${port}?`);
  }
}

/** Connects to the first page whose URL contains `match`. Returns { call, evaluate, close }. */
export async function connect({ match, port = DEFAULT_PORT, timeoutMs = 8000 }) {
  const targets = await listTargets(port);
  const t = targets.find((x) => x.type === 'page' && x.url.includes(match));
  if (!t) throw new Error(`no page matching "${match}". Pages:\n${targets.map((x) => `${x.type} ${x.url}`).join('\n')}`);
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error('websocket refused (start the app with --remote-allow-origins=*)'));
  });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const j = JSON.parse(m.data);
    const p = pending.get(j.id);
    if (!p) return;
    pending.delete(j.id);
    clearTimeout(p.timer);
    if (j.error) p.reject(new Error(j.error.message)); else p.resolve(j.result);
  };
  const call = (method, params = {}, ms = timeoutMs) =>
    new Promise((resolve, reject) => {
      const i = ++id;
      const timer = setTimeout(() => { pending.delete(i); reject(new Error(`timeout (${ms} ms) in ${method}`)); }, ms);
      pending.set(i, { resolve, reject, timer });
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  const evaluate = async (expression, ms = timeoutMs) => {
    const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, ms);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  };
  return { call, evaluate, close: () => ws.close() };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: { port: { type: 'string' }, timeout: { type: 'string' } },
  });
  const port = Number(values.port) || DEFAULT_PORT;
  const [match, expr] = positionals;
  try {
    if (match === 'list') {
      console.log((await listTargets(port)).map((t) => `${t.type} ${t.url}`).join('\n'));
    } else if (match && expr) {
      const page = await connect({ match, port });
      console.log(JSON.stringify(await page.evaluate(expr, Number(values.timeout) || 8000)));
      page.close();
    } else {
      console.error('usage: node cdp.mjs list | node cdp.mjs <url-substring> "<js expression>" [--port N] [--timeout ms]');
      process.exitCode = 2;
    }
  } catch (e) {
    console.error(e.message);
    process.exitCode = 1;
  }
  process.exit();
}
