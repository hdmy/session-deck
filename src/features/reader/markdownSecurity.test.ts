import { describe, expect, it } from 'vitest';
import { renderSafeMarkdown } from './markdownSecurity';

describe('renderSafeMarkdown', () => {
  it('renders basic Markdown while removing executable HTML', () => {
    const html = renderSafeMarkdown('# Safe\n\n<script>alert(1)</script>**bold**');
    expect(html).toContain('<h1>Safe</h1>');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).not.toContain('<script');
    expect(html).not.toContain('alert(1)');
  });

  it('does not let adjacent raw HTML swallow following Markdown', () => {
    const html = renderSafeMarkdown('<div>**bold after html**</div>');
    expect(html).toContain('<strong>bold after html</strong>');
    expect(html).not.toContain('<div>');
  });

  it('does not preserve remote or navigable resource attributes', () => {
    const html = renderSafeMarkdown('[remote](https://example.com)\n\n![pixel](https://example.com/a.png)\n\n<iframe src="https://example.com"></iframe>');
    expect(html).toContain('remote');
    expect(html).not.toContain('href=');
    expect(html).not.toContain('src=');
    expect(html).not.toContain('<img');
    expect(html).not.toContain('<iframe');
  });

  it('preserves HTML-like source inside fenced and inline code', () => {
    const html = renderSafeMarkdown(
      '```vue\n<template><img src="https://example.com/pixel.png"></template>\n```\n\nUse `<Component<T>>`.',
    );
    expect(html).toContain('&lt;template&gt;');
    expect(html).toContain('&lt;img src="https://example.com/pixel.png"&gt;');
    expect(html).toContain('&lt;Component&lt;T&gt;&gt;');
    expect(html).not.toContain('<img');
  });
});
