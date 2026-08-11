/** Keep control characters and traversal segments out of a displayed path. */
export function normalizeFilePath(value: string): string {
  const clean = value.replace(/[\u0000-\u001f\u007f]/g, '').replace(/\\/g, '/').trim();
  const absolute = clean.startsWith('/');
  const segments: string[] = [];
  for (const segment of clean.split('/')) {
    if (!segment || segment === '.') continue;
    if (segment === '..') {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  const normalized = `${absolute ? '/' : ''}${segments.join('/')}`;
  return normalized || '(unknown path)';
}

export function positiveCount(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}
