const BASE = 'http://localhost:8080';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };
async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email, password: 'password123', name: 'Empleado' }) });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const authed = (cookie) => (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) } });
const join = (projectId, name) => fetch(BASE + '/api/agent/join', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ projectId, name }) }).then((r) => r.json());
const bind = (b) => fetch(BASE + '/api/agent/bind', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(b) });

async function main() {
  const t = Date.now();
  const email = `emp_${t}@cli.com`;
  const u = await register(email);
  const fa = authed(u.cookie);
  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'Bind Co' }) })).json()).project;
  const pc1 = await join(proj.id, 'PC-1');
  const pc2 = await join(proj.id, 'PC-2');

  const roster = async () => (await (await fa(`/api/chat/empresas/${proj.id}/roster`)).json()).roster;
  const pcOf = (r, id) => r.find((x) => x.id === id);

  // Bind a PC1.
  const b1 = await bind({ equipoId: pc1.equipoId, agentSecret: pc1.agentSecret, email, password: 'password123' });
  const b1j = await b1.json();
  check(b1.status === 200 && b1j.userId === u.user.id, 'login en PC-1 casa usuario↔PC');
  let r = await roster();
  check(pcOf(r, pc1.equipoId)?.currentUserId === u.user.id, 'roster: PC-1 muestra al usuario casado');

  // Bind el MISMO usuario a PC2 → PC1 debe quedar libre (un PC por usuario).
  await bind({ equipoId: pc2.equipoId, agentSecret: pc2.agentSecret, email, password: 'password123' });
  r = await roster();
  check(pcOf(r, pc2.equipoId)?.currentUserId === u.user.id, 'PC-2 ahora tiene al usuario');
  check(pcOf(r, pc1.equipoId)?.currentUserId == null, 'PC-1 quedó libre (un usuario en un solo PC)');

  // Password incorrecta → 401.
  const bad = await bind({ equipoId: pc1.equipoId, agentSecret: pc1.agentSecret, email, password: 'malo' });
  check(bad.status === 401, 'password incorrecta → 401');

  // agentSecret incorrecto → 401.
  const bad2 = await bind({ equipoId: pc1.equipoId, agentSecret: 'x'.repeat(40), email, password: 'password123' });
  check(bad2.status === 401, 'agentSecret incorrecto → 401');

  // Unbind PC2.
  await fetch(BASE + '/api/agent/unbind', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ equipoId: pc2.equipoId, agentSecret: pc2.agentSecret }) });
  r = await roster();
  check(pcOf(r, pc2.equipoId)?.currentUserId == null, 'unbind libera PC-2');

  console.log(fail === 0 ? '\nTODOS LOS CHECKS DE BIND OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
