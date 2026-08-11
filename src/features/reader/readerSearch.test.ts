import { describe, expect, it, vi } from 'vitest';
import { findReaderMatches, nextMatchIndex, previousMatchIndex, scrollToReaderMatch } from './readerSearch';
import { highlightSanitizedHtml } from './markdownSecurity';

describe('reader search helpers', () => {
  it('matches Unicode text case-insensitively and preserves match positions', () => {
    expect(findReaderMatches('Résumé RÉSUMÉ 中文', 'résumé', 8).map((match) => match.eventId)).toEqual([8, 8]);
    expect(findReaderMatches('Résumé RÉSUMÉ 中文', '中文', 8)[0]?.start).toBe(14);
  });

  it('wraps next and previous indexes', () => {
    expect(nextMatchIndex(-1, 3)).toBe(0);
    expect(nextMatchIndex(2, 3)).toBe(0);
    expect(previousMatchIndex(0, 3)).toBe(2);
    expect(previousMatchIndex(0, 0)).toBe(-1);
  });

  it('scrolls only stable event ids', () => {
    const element = document.createElement('article');
    element.id = 'event-4';
    element.scrollIntoView = vi.fn();
    document.body.append(element);
    expect(scrollToReaderMatch({ eventId: 4, start: 0, end: 1, text: 'x' })).toBe(true);
    expect(element.scrollIntoView).toHaveBeenCalled();
    expect(scrollToReaderMatch({ eventId: 5, start: 0, end: 1, text: 'x' })).toBe(false);
    element.remove();
  });

  it('opens collapsed reader disclosures before scrolling a hidden activity match', () => {
    const reader = document.createElement('main');
    reader.className = 'reader';
    const turnActivity = document.createElement('details');
    const collapsedEvent = document.createElement('details');
    const element = document.createElement('article');
    element.id = 'event-6';
    element.scrollIntoView = vi.fn();
    collapsedEvent.append(element);
    turnActivity.append(collapsedEvent);
    reader.append(turnActivity);
    document.body.append(reader);

    expect(scrollToReaderMatch({ eventId: 6, start: 0, end: 1, text: 'tool' })).toBe(true);
    expect(turnActivity.open).toBe(true);
    expect(collapsedEvent.open).toBe(true);
    expect(element.scrollIntoView).toHaveBeenCalled();
    reader.remove();
  });
});

describe('safe search highlighting', () => {
  it('highlights after sanitization without reviving scripts or resource attributes', () => {
    const html = highlightSanitizedHtml('<p>Hello <strong>World</strong></p>', 'world');
    expect(html).toContain('<mark class="reader-highlight">World</mark>');
    const hostile = highlightSanitizedHtml('<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>', 'script');
    expect(hostile).not.toContain('<script');
    expect(hostile).not.toContain('href=');
    expect(hostile).not.toContain('src=');
  });
});
