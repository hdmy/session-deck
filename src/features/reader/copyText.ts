/**
 * Copy text without exposing a shell or retaining a DOM node after the
 * operation. Clipboard permissions are best-effort: older WebViews can still
 * support the synchronous textarea path.
 */
export function fallbackCopyText(value: string): boolean {
  if (typeof document === 'undefined' || !document.body) return false;
  const textarea = document.createElement('textarea');
  const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  textarea.value = value;
  textarea.setAttribute('readonly', '');
  textarea.setAttribute('aria-hidden', 'true');
  textarea.tabIndex = -1;
  textarea.style.position = 'fixed';
  textarea.style.left = '-9999px';
  textarea.style.top = '0';
  textarea.style.opacity = '0';
  try {
    document.body.append(textarea);
    textarea.select();
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    textarea.remove();
    try { previousFocus?.focus({ preventScroll: true }); } catch { /* focus restoration is best effort */ }
  }
}

export async function copyText(value: string): Promise<boolean> {
  try {
    const clipboard = typeof navigator !== 'undefined' ? navigator.clipboard : undefined;
    if (clipboard?.writeText) {
      await clipboard.writeText.call(clipboard, value);
      return true;
    }
  } catch {
    // A rejected permission request falls through to the WebView-safe path.
  }
  return fallbackCopyText(value);
}
