// Avatares: iniciales + color estable derivado del identificador.

const PALETTE = ['#4a90e2', '#e2845b', '#6ee7a8', '#b57edc', '#e2c15b', '#5bc8e2', '#e25b8f', '#7e8be2'];

export function avatarColor(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
