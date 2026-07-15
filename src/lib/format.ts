export function bytes(n: number): string {
  if (n < 1000) return `${Math.round(n)} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1000;
  let i = 0;
  while (v >= 1000 && i < u.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${u[i]}`;
}

export function rate(bps: number): string {
  return `${bytes(bps)}/s`;
}

export function count(n: number): string {
  return n.toLocaleString("en-US");
}

export function eta(bytesLeft: number, bps: number): string {
  if (bps <= 0) return "—";
  const s = bytesLeft / bps;
  if (s < 60) return `${Math.ceil(s)}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${Math.round((s % 3600) / 60)}m`;
}
