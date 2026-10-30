// Starts a private headless Chrome / Edge (no window, own profile in the temp dir) and hands back a CDP page.
// Used by export-pets.mjs and pets-diff.mjs. Needs Node 22+ (global WebSocket / fetch). Nothing is written outside the temp dir.
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { connect } from './cdp.mjs';

const CANDIDATES = [
  process.env.CHROME,
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
].filter(Boolean);

export async function launch({ port = 9444 + Math.floor(Math.random() * 400), url = 'about:blank', width = 800, height = 600, timeoutMs = 60000 } = {}) {
  const exe = CANDIDATES.find((p) => existsSync(p));
  if (!exe) throw new Error('no Chrome / Edge found (set CHROME=path)');
  const dir = mkdtempSync(join(tmpdir(), 'pets-chrome-'));
  const child = spawn(exe, [
    '--headless=new', '--disable-gpu', '--hide-scrollbars', '--no-first-run', '--no-default-browser-check', '--allow-file-access-from-files',
    '--force-device-scale-factor=1', `--window-size=${width},${height}`, `--user-data-dir=${dir}`,
    `--remote-debugging-port=${port}`, '--remote-allow-origins=*', url,
  ], { stdio: 'ignore', windowsHide: true });
  let page = null;
  for (let i = 0; i < 100 && !page; i++) {
    try { page = await connect({ match: '', port, timeoutMs }); } catch { await new Promise((r) => setTimeout(r, 200)); }
  }
  if (!page) { child.kill(); throw new Error('Chrome did not expose the debug port'); }
  const close = async () => {
    try { page.close(); } catch { /* ignore */ }
    child.kill();
    await new Promise((r) => setTimeout(r, 300));
    try { rmSync(dir, { recursive: true, force: true }); } catch { /* the profile lock may linger; the temp dir is cleaned by the OS */ }
  };
  return { ...page, close, port };
}
