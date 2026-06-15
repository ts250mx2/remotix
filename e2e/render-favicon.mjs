import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const svg = readFileSync(path.join(here, '..', 'web', 'public', 'favicon.svg'), 'utf8');

const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 260, height: 130 } });
await p.setContent(`<body style="margin:0;display:flex;gap:24px;align-items:center;justify-content:center;background:#f1f3f8;height:100vh;font-family:sans-serif">
  <div style="width:96px;height:96px">${svg}</div>
  <div style="width:48px;height:48px">${svg}</div>
  <div style="width:24px;height:24px">${svg}</div>
  <div style="width:16px;height:16px">${svg}</div>
</body>`);
await p.screenshot({ path: path.join(here, 'fav-render.png') });
await b.close();
console.log('ok');
