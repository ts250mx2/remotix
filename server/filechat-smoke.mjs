const BASE = 'http://localhost:8080';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };

async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: 'password123', name: 'Ana' }),
  });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const authed = (cookie) => (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie, ...(o.body && typeof o.body === 'string' ? { 'content-type': 'application/json' } : {}) } });

async function main() {
  const A = await register(`f_${Date.now()}@t.com`);
  const fa = authed(A.cookie);
  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'Files Co' }) })).json()).project;
  const channels = (await (await fa(`/api/chat/channels?empresa_id=${proj.id}`)).json()).channels;
  const general = channels[0];

  const content = 'archivo de prueba\n' + 'L'.repeat(3000);
  const fd = new FormData();
  fd.append('file', new File([content], 'notas.txt', { type: 'text/plain' }));
  fd.append('body', 'mira estas notas');

  const upRes = await fa(`/api/chat/channels/${general.id}/files`, { method: 'POST', body: fd });
  const up = await upRes.json();
  check(upRes.status === 201 && up.message?.attachment?.name === 'notas.txt' && up.message.body === 'mira estas notas',
    `subida con adjunto (${up.message?.attachment?.size} bytes)`);

  const dl = await fa(`/api/chat/files/${up.message.id}`);
  const text = await dl.text();
  check(dl.status === 200 && text === content, 'descarga del adjunto con contenido íntegro');
  check((dl.headers.get('content-disposition') || '').includes('notas.txt'), 'content-disposition con nombre de archivo');

  const hist = (await (await fa(`/api/chat/channels/${general.id}/messages`)).json()).messages;
  check(hist.some((m) => m.attachment?.name === 'notas.txt'), 'adjunto aparece en el historial');

  console.log(fail === 0 ? '\nTODOS LOS CHECKS DE ARCHIVOS-CHAT OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
