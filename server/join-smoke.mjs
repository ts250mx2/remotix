const BASE = 'http://localhost:8080';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };

async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: 'password123', name: email.split('@')[0] }),
  });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const authed = (cookie) => (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) } });

async function main() {
  const t = Date.now();
  const admin = await register(`admin_${t}@msp.com`);
  const tec = await register(`tec_${t}@msp.com`);
  const fa = authed(admin.cookie);

  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'ACME Cliente' }) })).json()).project;
  check(/^py_/.test(proj.id), `empresa creada (UUID ${proj.id})`);

  // Unir un PC SOLO con el UUID (sin auth, sin pairing code).
  const joinRes = await fetch(BASE + '/api/agent/join', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ projectId: proj.id, name: 'PC-Recepcion', os: 'windows', hostname: 'RECEP01' }),
  });
  const join = await joinRes.json();
  check(joinRes.status === 201 && /^eq_/.test(join.equipoId) && !!join.agentSecret, `PC unido por UUID (${join.equipoId})`);

  // UUID inválido → rechazado.
  const bad = await fetch(BASE + '/api/agent/join', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ projectId: 'py_' + 'x'.repeat(22), name: 'X' }),
  });
  check(bad.status === 400, 'UUID inválido rechazado');

  // Lookup del técnico por email + alta como técnico.
  const look = await (await fa(`/api/users/lookup?email=${encodeURIComponent(`tec_${t}@msp.com`)}`)).json();
  check(look.user?.id === tec.user.id, 'lookup de técnico por email');
  const add = await fa(`/api/projects/${proj.id}/members`, { method: 'POST', body: JSON.stringify({ principalId: tec.user.id, role: 'tecnico' }) });
  check(add.status === 200, 'técnico asignado a la empresa');

  // Roster: debe incluir el PC y el técnico.
  const roster = (await (await fa(`/api/chat/empresas/${proj.id}/roster`)).json()).roster;
  const hasPc = roster.some((r) => r.kind === 'pc' && r.name === 'PC-Recepcion');
  const hasTec = roster.some((r) => r.kind === 'user' && r.role === 'tecnico');
  check(hasPc && hasTec, `roster incluye PC y técnico (${roster.length} entradas)`);

  console.log(fail === 0 ? '\nTODOS LOS CHECKS OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
