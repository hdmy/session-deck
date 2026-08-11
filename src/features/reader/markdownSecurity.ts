import DOMPurify from 'dompurify';
import { marked } from 'marked';

const allowedTags = [
  'p', 'br', 'strong', 'em', 'del', 's', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'ul', 'ol', 'li', 'blockquote', 'pre', 'code', 'table', 'thead', 'tbody',
  'tr', 'th', 'td', 'hr', 'a',
];

function neutralizeRawHtml(markdown: string): string {
  const protectedSegments: string[] = [];
  const protect = (value: string) => {
    const token = `\uE000CONTEXT_VAULT_CODE_${protectedSegments.length}\uE001`;
    protectedSegments.push(value);
    return token;
  };
  const lines = markdown.match(/[^\n]*(?:\n|$)/g)?.filter(Boolean) ?? [];
  const protectedMarkdown: string[] = [];
  let fence: { marker: '`' | '~'; length: number; lines: string[] } | null = null;

  for (const line of lines) {
    const match = line.match(/^ {0,3}([`~]{3,})([^\r\n]*)/);
    const marker = match?.[1];
    const validMarker = marker && [...marker].every((character) => character === marker[0]);
    if (fence) {
      fence.lines.push(line);
      if (
        validMarker
        && marker[0] === fence.marker
        && marker.length >= fence.length
        && (match?.[2] ?? '').trim() === ''
      ) {
        protectedMarkdown.push(protect(fence.lines.join('')));
        fence = null;
      }
      continue;
    }
    if (validMarker) {
      fence = { marker: marker[0] as '`' | '~', length: marker.length, lines: [line] };
      continue;
    }
    protectedMarkdown.push(protectInlineCode(line, protect));
  }
  if (fence) protectedMarkdown.push(protect(fence.lines.join('')));

  let neutralized = protectedMarkdown.join('')
    .replace(/<\s*(script|style|iframe|object|embed|form|svg|math)\b[^>]*>[\s\S]*?<\s*\/\s*\1\s*>/gi, '')
    .replace(/<[^>]+>/g, '');
  protectedSegments.forEach((segment, index) => {
    neutralized = neutralized.replaceAll(`\uE000CONTEXT_VAULT_CODE_${index}\uE001`, segment);
  });
  return neutralized;
}

function protectInlineCode(line: string, protect: (value: string) => string): string {
  let output = '';
  let cursor = 0;
  while (cursor < line.length) {
    const opening = line.indexOf('`', cursor);
    if (opening < 0) return output + line.slice(cursor);
    output += line.slice(cursor, opening);
    let length = 1;
    while (line[opening + length] === '`') length += 1;
    const delimiter = '`'.repeat(length);
    const closing = line.indexOf(delimiter, opening + length);
    if (closing < 0) return output + line.slice(opening);
    output += protect(line.slice(opening, closing + length));
    cursor = closing + length;
  }
  return output;
}

/** Converts transcript Markdown to inert, local-only HTML. Links are text-only to prevent navigation. */
export function renderSafeMarkdown(markdown: string): string {
  const parsed = marked.parse(neutralizeRawHtml(markdown), { async: false, gfm: true, breaks: true });
  const html = typeof parsed === 'string' ? parsed : '';
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: allowedTags,
    ALLOWED_ATTR: [],
    FORBID_ATTR: ['src', 'srcset', 'href', 'action', 'formaction', 'style', 'target'],
    FORBID_TAGS: ['script', 'style', 'img', 'picture', 'source', 'iframe', 'frame', 'object', 'embed', 'audio', 'video', 'form'],
    KEEP_CONTENT: true,
  });
}

/** Highlight text nodes only after Markdown has been sanitized. The query is
 * never concatenated into HTML, so it cannot create tags or attributes. */
export function highlightSanitizedHtml(html: string, query: string): string {
  const trimmed = query.trim();
  if (!trimmed || typeof document === 'undefined') return html;
  const root = document.createElement('div');
  root.innerHTML = html;
  const expression = new RegExp(escapeRegExp(trimmed), 'giu');
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  let node: Node | null;
  while ((node = walker.nextNode())) {
    if (node.parentElement?.closest('script,style,mark')) continue;
    if (node.textContent && expression.test(node.textContent)) nodes.push(node as Text);
    expression.lastIndex = 0;
  }
  for (const textNode of nodes) {
    const text = textNode.textContent ?? '';
    const fragment = document.createDocumentFragment();
    let cursor = 0;
    expression.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = expression.exec(text))) {
      if (match.index > cursor) fragment.append(text.slice(cursor, match.index));
      const mark = document.createElement('mark');
      mark.className = 'reader-highlight';
      mark.textContent = match[0];
      fragment.append(mark);
      cursor = match.index + match[0].length;
      if (!match[0].length) expression.lastIndex += 1;
    }
    if (cursor < text.length) fragment.append(text.slice(cursor));
    textNode.replaceWith(fragment);
  }
  return root.innerHTML;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
