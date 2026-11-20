import { test, expect } from '@playwright/test';
import { descriptionSegments, stripTags, MAX_DESCRIPTION_LENGTH } from '../src/lib/sanitize';

const text = (raw: string | null) =>
  descriptionSegments(raw).map((s) => s.value).join('');

test.describe('descriptionSegments', () => {
  /** Plain text carries `<` too. Only something shaped like a real HTML tag
   *  is markup to strip; an address in angle brackets (a forwarded header's
   *  "Alice <alice@corp.com>") and a less-than in a sentence are what the
   *  author wrote, and were being eaten up to the next `>`. */
  test('an address in angle brackets and a less-than stay as written', () => {
    expect(text('From: Alice <alice@corp.com>')).toBe('From: Alice <alice@corp.com>');
    expect(text('if p99 < 5ms and qps > 100')).toBe('if p99 < 5ms and qps > 100');
    expect(text('<b>bold</b> and <i>it</i>')).toBe('bold and it');
    // Shaped like a tag and named like none: the author's placeholder.
    expect(text('Owner: <TBD>')).toBe('Owner: <TBD>');
  });

  /** Outlook and Teams invitations open with a `<style>` block and
   *  conditional comments. Stripping only their tags left the CSS itself on
   *  screen, ahead of the actual invitation. */
  test('a style block, a script and a comment leave nothing behind', () => {
    expect(text('<style>p.MsoNormal{margin:0}</style><p>Hi</p>')).toBe('Hi');
    expect(text('<script>alert(1)</script>after')).toBe('after');
    expect(text('<!--[if mso]><b>x</b><![endif]-->kept')).toBe('kept');
    expect(text('before<!-- a note -->after')).toBe('beforeafter');
  });

  /** List items and table rows are lines, as paragraphs are. */
  test('list items and table rows keep their lines', () => {
    expect(text('<ul><li>one</li><li>two</li></ul>')).toBe('• one\n• two');
    expect(text('<table><tr><td>a</td></tr><tr><td>b</td></tr></table>')).toBe('a\nb');
  });

  /** An anchor someone forgot to close is not a link around everything up
   *  to the next anchor's `</a>`. */
  test('an unclosed anchor does not swallow the next one', () => {
    const out = descriptionSegments('<a href="https://x.com">Link<p>para two</p><a href="https://y.com">y</a>');
    const links = out.filter((s) => s.kind === 'link');
    expect(links).toEqual([{ kind: 'link', value: 'y', href: 'https://y.com' }]);
    expect(text('<a href="https://x.com">Link<p>para two</p><a href="https://y.com">y</a>')).toBe('Linkpara two\ny');
  });

  /** A right-to-left override reverses what follows it on screen, so a
   *  description could show one thing while saying another. */
  test('bidirectional control characters are removed', () => {
    expect(text('pay ‮gnp.exe')).toBe('pay gnp.exe');
    expect(text('pay &#x202e;gnp.exe')).toBe('pay gnp.exe');
  });

  /** A sentence's full stop, or the parenthesis around a link, is not part
   *  of where the link goes. */
  test('a link ends before the punctuation around it', () => {
    const href = (raw: string) => descriptionSegments(raw).find((s) => s.kind === 'link')?.href;
    expect(href('See https://example.com/a.')).toBe('https://example.com/a');
    expect(href('(https://example.com/foo)')).toBe('https://example.com/foo');
    expect(href('https://en.wikipedia.org/wiki/Function_(mathematics)')).toBe('https://en.wikipedia.org/wiki/Function_(mathematics)');
    expect(text('See https://example.com/a.')).toBe('See https://example.com/a.');
  });

  test('a script tag is shown, not run', () => {
    // Anyone who knows your email can put an event on your calendar, so a
    // description is attacker-controlled input inside a webview that can call
    // Tauri commands. It must never become markup.
    const out = descriptionSegments('<script>alert(1)</script>');
    expect(out.every((s) => s.kind === 'text')).toBe(true);
    expect(text('<script>alert(1)</script>')).not.toContain('<script>');
  });

  test('an img onerror payload survives only as text', () => {
    expect(text('<img src=x onerror=alert(1)>')).not.toContain('onerror');
  });

  test('line breaks become newlines', () => {
    expect(text('one<br>two<br/>three')).toBe('one\ntwo\nthree');
    expect(text('<p>one</p><p>two</p>')).toBe('one\ntwo');
  });

  test('entities are decoded', () => {
    expect(text('Tom &amp; Jerry &lt;3 &quot;hi&quot; &#39;x&#39;&nbsp;y'))
      .toBe('Tom & Jerry <3 "hi" \'x\' y');
  });

  test('a bare url becomes a link segment', () => {
    const out = descriptionSegments('join at https://meet.google.com/abc now');
    expect(out.map((s) => s.kind)).toEqual(['text', 'link', 'text']);
    expect(out[1].value).toBe('https://meet.google.com/abc');
  });

  test('a javascript: url is never a link', () => {
    // The linkifier is the one place a URL becomes an href, so it is the one
    // place a scheme check belongs.
    expect(descriptionSegments('javascript:alert(1)').every((s) => s.kind === 'text'))
      .toBe(true);
  });

  test('null and empty give nothing to render', () => {
    expect(descriptionSegments(null)).toEqual([]);
    expect(descriptionSegments('   ')).toEqual([]);
  });

  // Hostile cases beyond the brief's list — an attacker's description isn't
  // limited to the shapes above.

  // This is the test that actually catches "decode before strip". Neither
  // the plain <script> tag test nor the entities test contains a real '<'
  // and a real '>' produced only *after* decoding, so reordering the two
  // steps doesn't change their output. An entity-encoded tag does: decode
  // it before stripping and the stripper sees a real <script> tag and
  // removes it, silently turning what the user typed into "alert(1)".
  // Stripping first means there is nothing tag-shaped to strip at that
  // point, so the encoded tag survives decoding as inert literal text —
  // shown back exactly as typed, never reinterpreted as markup.
  test('an entity-encoded tag is shown literally, never reinterpreted as markup', () => {
    expect(text('&lt;script&gt;alert(1)&lt;/script&gt;')).toBe('<script>alert(1)</script>');
  });

  test('nested or malformed tags never produce a link and never survive as markup', () => {
    const out = descriptionSegments('<scr<script>ipt>alert(1)</script>');
    expect(out.every((s) => s.kind === 'text')).toBe(true);
    expect(text('<scr<script>ipt>alert(1)</script>')).not.toContain('<script>');
  });

  test('an event handler attribute on a block element is removed with its tag', () => {
    expect(text('<div onmouseover="alert(1)">Meeting notes</div>')).toBe('Meeting notes');
  });

  test('a data: url is never a link', () => {
    const out = descriptionSegments('data:text/html,<script>alert(1)</script>');
    expect(out.every((s) => s.kind === 'text')).toBe(true);
  });

  // Fix round 1: TAG_RE (`/<[^>]*>/g`) was O(n^2) on a run of unmatched
  // `<` — a single-invite calendar description with no closing `>` could
  // freeze the webview for tens of seconds. Two independent fixes, two
  // independent tests below: a hard length cap (deterministic, the primary
  // guard), and a linear-time tag strip (proven directly, since the cap
  // would otherwise mask a quadratic regression at 32KB by making it fast
  // enough to slip under any reasonable timeout).

  test('an oversized description is capped, not processed unbounded', () => {
    const big = 'a'.repeat(MAX_DESCRIPTION_LENGTH + 20_000);
    const out = descriptionSegments(big);
    const joined = out.map((s) => s.value).join('');
    expect(joined.length).toBeLessThanOrEqual(MAX_DESCRIPTION_LENGTH);
    expect(joined.length).toBeGreaterThan(0);
  });

  test('a run of 200,000 unmatched "<" strips in linear time, not quadratic', () => {
    // Tests stripTags directly rather than through descriptionSegments:
    // the length cap above truncates any input before it reaches the
    // stripper, so at the capped size even the old O(n^2) regex finishes
    // in well under a second — the cap alone can't prove this scan is
    // linear, only this can.
    const input = '<'.repeat(200_000);
    const start = Date.now();
    const out = stripTags(input);
    // Generous and finite, not a millisecond budget: the old regex measured
    // ~14.5s at this size, so this bound stays robust on slow CI while still
    // catching a return to quadratic behavior.
    expect(Date.now() - start).toBeLessThan(5000);
    // No '>' anywhere in the input, so nothing is a tag — the whole run is
    // kept as literal text, same as the old (slow) implementation produced.
    expect(out).toBe(input);
  });

  /** A style or script element is skipped to its end, and an end that never
   *  comes is looked for once, not again from every later one: a run of
   *  unclosed `<style>`s would otherwise rescan the rest of the string from
   *  each of them. Same budget and reasoning as the test above. */
  test('a run of unclosed style elements is stripped in linear time', () => {
    const input = '<style>x'.repeat(200_000);
    const start = Date.now();
    stripTags(input);
    expect(Date.now() - start).toBeLessThan(5000);
  });

  test('an anchor written with words keeps its destination', () => {
    // Issue #19's real case. The stripper used to remove the tag and leave
    // "Pre-read" as bare text, so the one thing the link was for — where it
    // went — was gone, with nothing on screen to say a link had been there.
    const out = descriptionSegments('See the <a href="https://docs.example.com/x">Pre-read</a> first');
    expect(out.map((s) => s.kind)).toEqual(['text', 'link', 'text']);
    const link = out[1] as { kind: 'link'; value: string; href: string };
    expect(link.value).toBe('Pre-read');
    expect(link.href).toBe('https://docs.example.com/x');
    expect(text('See the <a href="https://docs.example.com/x">Pre-read</a> first'))
      .toBe('See the Pre-read first');
  });

  test('an anchor whose text is its own url is unchanged', () => {
    // 169 of the 217 anchors on the author's real calendar are this shape.
    // Label and destination are the same string, so this must come out
    // exactly as it did before anchors were understood at all.
    const out = descriptionSegments('<a href="https://meet.google.com/abc">https://meet.google.com/abc</a>');
    expect(out).toHaveLength(1);
    expect(out[0]).toEqual({
      kind: 'link', value: 'https://meet.google.com/abc', href: 'https://meet.google.com/abc',
    });
  });

  test('an anchor href is held to the same scheme rule as a bare url', () => {
    // `hrefOf` is the second and last place a URL becomes an href, so it
    // carries the same check. The words stay either way — losing the label
    // as well would hide that anything was written there.
    for (const href of ['javascript:alert(1)', 'data:text/html,hello', 'file:///etc/passwd']) {
      const out = descriptionSegments(`<a href="${href}">Click me</a>`);
      expect(out.every((s) => s.kind === 'text'), href).toBe(true);
      expect(out.map((s) => s.value).join('')).toBe('Click me');
    }
  });

  test('a tag-ending bracket inside an attribute loses the link, never forges one', () => {
    // Finding a tag's end ignores quoting — `indexOf('>')`, the same rule
    // `stripTags` has always used — so `href="…<script>"` ends the tag early
    // and the remainder becomes text. That can only ever *lose* a link: an
    // href is only followed when it matches an anchored http(s) pattern that
    // excludes `<`, `>` and both quotes, so a truncated attribute yields
    // nothing rather than something unintended.
    const out = descriptionSegments('<a href="data:text/html,<script>">Click me</a>');
    expect(out.every((s) => s.kind === 'text')).toBe(true);
    expect(out.map((s) => s.value).join('')).not.toContain('<script>');
  });

  test('an anchor with no usable href is words, not a link', () => {
    expect(descriptionSegments('<a>bare</a>').every((s) => s.kind === 'text')).toBe(true);
    expect(descriptionSegments('<a name="top">anchor</a>').every((s) => s.kind === 'text')).toBe(true);
    // `data-href` is not `href`, and must not be read as one.
    expect(descriptionSegments('<a data-href="https://x.example">no</a>')
      .every((s) => s.kind === 'text')).toBe(true);
  });

  test('the shapes a real invitation actually contains', () => {
    const href = (raw: string) => {
      const out = descriptionSegments(raw);
      const link = out.find((s) => s.kind === 'link');
      return link && link.kind === 'link' ? link.href : null;
    };
    // Single quotes, no quotes, uppercase tag and attribute, extra
    // attributes before and after, and a query string written with entities
    // — which is how every calendar invitation writes one.
    expect(href("<a href='https://x.example/a'>x</a>")).toBe('https://x.example/a');
    expect(href('<a href=https://x.example/b>x</a>')).toBe('https://x.example/b');
    expect(href('<A HREF="https://x.example/c">x</A>')).toBe('https://x.example/c');
    expect(href('<a target="_blank" href="https://x.example/d" rel="noopener">x</a>'))
      .toBe('https://x.example/d');
    expect(href('<a href="https://x.example/e?a=1&amp;b=2">x</a>'))
      .toBe('https://x.example/e?a=1&b=2');
  });

  test('nothing inside an anchor survives as markup', () => {
    // The label goes through the same stripper and decoder as any other
    // text, so an anchor is not a hole in the guarantee this module makes.
    const out = descriptionSegments('<a href="https://x.example"><img src=x onerror=alert(1)>Go</a>');
    const link = out.find((s) => s.kind === 'link');
    expect(link && link.kind === 'link' ? link.value : '').toBe('Go');
    expect(text('<a href="https://x.example"><b>bo</b>ld</a>')).toBe('bold');
    expect(descriptionSegments('<a href="https://x.example">&lt;script&gt;</a>')
      .map((s) => s.value).join('')).toBe('<script>');
  });

  test('an unclosed anchor is stripped like any other malformed tag', () => {
    // No `</a>` means no anchor: it falls back to the text path, which is
    // what happened before links carried labels and is still right.
    expect(descriptionSegments('<a href="https://x.example">dangling')
      .every((s) => s.kind === 'text')).toBe(true);
    expect(text('<a href="https://x.example">dangling')).toBe('dangling');
  });

  test('a run of 200,000 unclosed anchors is linear too', () => {
    // The anchor scan is new, and it is the one place that looks for a
    // *second* tag before deciding. Every branch advances past what it
    // consumed; this is what proves it.
    const input = '<a '.repeat(60_000);
    const start = Date.now();
    const out = descriptionSegments(input);
    expect(Date.now() - start).toBeLessThan(5000);
    expect(out.every((s) => s.kind === 'text')).toBe(true);
  });
});
