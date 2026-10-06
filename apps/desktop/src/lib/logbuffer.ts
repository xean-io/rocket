/** Appends `incoming` to `lines`, keeping only the newest `cap` lines. */
export function appendCapped(lines: string[], incoming: string[], cap: number): string[] {
  if (incoming.length === 0) return lines;
  const merged = lines.length === 0 ? incoming : lines.concat(incoming);
  return merged.length > cap ? merged.slice(merged.length - cap) : merged;
}
