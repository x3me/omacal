// Anyone who knows a user's email address can put an event on their
// calendar, description included. That description renders inside a webview
// that can invoke Tauri commands, so it is attacker-controlled input, not
// display text. This module never produces HTML: it returns segments that
// the component renders with `{#each}` and a plain `<a>`, so there is no
// code path where `{@html}` could be reintroduced by a later edit.

/**
 * What the component renders: `value` is always the text shown, and a link
 * additionally carries where it goes.
 *
 * The two are separate fields because they are separate facts. A bare URL in
 * a description is its own label and they are equal; an anchor written with
 * words — `<a href="…">Pre-read</a>` — has a label that is not a URL, and
 * flattening it to text was dropping the destination entirely. Keeping them
 * apart is what lets that link survive without this module ever producing
 * markup: the destination is data on a segment, not an attribute in a string.
 */
export type Segment =
  | { kind: 'text'; value: string }
  | { kind: 'link'; value: string; href: string };

// Step 1: these are the only tags treated as line structure, converted to
// newlines while they are still real markup. A list item and a table row are
// lines, as a paragraph is. Bounded (`{0,200}`), never `[^>]*`: an unbounded
// class on a run of unclosed `<li` would backtrack to the end of the string
// from every one of them (see stripTags).
const BREAK_RE = /<br\s*\/?>|<\/p>|<\/div>|<\/li>|<\/tr>|<\/h[1-6]>/gi;
const ITEM_RE = /<li(?:\s[^<>]{0,200})?>/gi;

// The tag names this module treats as markup. Anything else after a `<` is
// what the author wrote: an address in angle brackets ("Alice
// <alice@corp.com>"), or a less-than in a sentence. A namespaced name
// (`<o:p>`, `<w:WordDocument>`) is Office's markup, and counts too.
const TAGS = new Set([
  'a', 'abbr', 'b', 'big', 'blockquote', 'body', 'br', 'caption', 'center', 'cite', 'code', 'col',
  'colgroup', 'dd', 'del', 'dfn', 'div', 'dl', 'dt', 'em', 'font', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'head', 'hr', 'html', 'i', 'img', 'ins', 'kbd', 'li', 'link', 'mark', 'meta', 'ol', 'p', 'pre', 'q',
  's', 'samp', 'script', 'section', 'small', 'span', 'strike', 'strong', 'style', 'sub', 'sup',
  'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'title', 'tr', 'tt', 'u', 'ul', 'var', 'wbr', 'xml',
]);

// Characters that reorder what follows them on screen (embeddings,
// overrides, isolates). A description could otherwise show one thing while
// saying another: `pay \u202Egnp.exe` reads as "pay exe.png".
const BIDI_RE = /[\u202A-\u202E\u2066-\u2069]/g;

// Step 4: the only place a URL becomes an href, so the only place a scheme
// check belongs. http/https only — a javascript: or data: URL never matches.
const URL_RE = /https?:\/\/[^\s<>"']+/gi;

// A description longer than this is not something anyone reads in a
// popover, and it bounds the input to every stage below, not just the tag
// stripper — one fix against every present and future blowup in this
// function, not only the one found in review (see stripTags). Truncate
// rather than reject: the popover should still show what fits.
export const MAX_DESCRIPTION_LENGTH = 32 * 1024;

// Step 2: everything else that looks like a tag is removed outright, in a
// single forward pass. This runs on the raw markup, before any entity
// decoding — see decodeEntities below for why the order matters.
//
// This used to be `input.replace(/<[^>]*>/g, '')`. On a run of unmatched
// `<` (e.g. an attacker-sized `'<'.repeat(200000)`), that regex's greedy
// `[^>]*` runs to end-of-string, fails to find the required `>`, backtracks
// a character at a time, and then the `g` flag restarts the whole attempt
// one position later — O(n²), measured at 14.5s for 200,000 characters.
// No regex on this path can be made obviously non-backtracking, and
// "obviously" is what a security boundary needs, so this walks the string
// once instead.
//
// Exported only so its linear-time behavior can be exercised directly by a
// large-input test. With MAX_DESCRIPTION_LENGTH applied first in
// descriptionSegments, an adversarial input never reaches this function at
// its full size, so testing only through descriptionSegments couldn't tell
// a fixed scan from a quadratic one that just happens to be fast at 32KB.
// Its output is plain stripped text — not entity-decoded, not link-checked —
// so it is *not* a general-purpose safe-HTML utility: never pass its result
// to `{@html}`. Always render a description through descriptionSegments,
// which runs this as one step among several and returns segments a
// component renders with `{#each}` instead.
export function stripTags(input: string): string {
  let out = '';
  let i = 0;
  const n = input.length;
  // Searched for once each: an element whose end never comes is not looked
  // for again from every later one, which is what keeps this linear on an
  // input of a thousand unclosed `<style>`s.
  const unclosed = new Set<string>();
  while (i < n) {
    if (input[i] !== '<') {
      out += input[i];
      i++;
      continue;
    }
    // A comment, conditional or not, goes whole: Outlook's `<!--[if mso]>`
    // blocks hold markup of their own, which must not surface either.
    if (input.startsWith('<!--', i)) {
      const end = unclosed.has('-->') ? -1 : input.indexOf('-->', i + 4);
      if (end === -1) { unclosed.add('-->'); break; }
      i = end + 3;
      continue;
    }
    // A declaration or processing instruction: `<!DOCTYPE>`, `<![endif]>`,
    // `<?xml>`.
    if (input[i + 1] === '!' || input[i + 1] === '?') {
      const close = input.indexOf('>', i + 2);
      if (close === -1) { out += input.slice(i); break; }
      i = close + 1;
      continue;
    }
    let j = i + 1;
    const closing = input[j] === '/';
    if (closing) j++;
    const nameStart = j;
    while (j < n && j - nameStart < 40 && /[A-Za-z0-9:-]/.test(input[j])) j++;
    const name = input.slice(nameStart, j).toLowerCase();
    const shaped = /^[a-z]/.test(name) && (j >= n || /[\s/>]/.test(input[j]));
    if (!shaped || !(TAGS.has(name) || name.includes(':'))) {
      // Not markup: the `<` is the author's, kept as written.
      out += '<';
      i++;
      continue;
    }
    const close = input.indexOf('>', j);
    if (close === -1) {
      // Nothing from here on can close a tag, so nothing from here on
      // can be one either — keep the remainder as literal text and stop.
      // This is what keeps the scan linear instead of restarting a search at
      // every subsequent `<`.
      out += input.slice(i);
      break;
    }
    i = close + 1; // drop the whole <...> run, including both brackets
    // A style sheet or a script is not text: its body goes with its tags.
    // Outlook and Teams invitations open with a `<style>` block, and only
    // the tags used to go, leaving the CSS on screen.
    if (!closing && (name === 'style' || name === 'script') && !unclosed.has(name)) {
      const endRe = new RegExp(`</${name}\\s*>`, 'gi');
      endRe.lastIndex = i;
      const m = endRe.exec(input);
      if (m) i = m.index + m[0].length;
      else unclosed.add(name);
    }
  }
  return out;
}

const NAMED_ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
};

// Step 3: decode entities only after tags are stripped. Decoding first would
// turn `&lt;script&gt;` into a real `<script>` tag that the stripper then
// either removes as markup (silently changing what the user typed) or,
// depending on how it strips, leaves as live-looking markup. Running this
// after stripping means an encoded tag can only ever end up as literal,
// inert text — exactly what was typed, shown back rather than reinterpreted.
function decodeEntities(input: string): string {
  return input.replace(/&(#x?[0-9a-f]+|[a-z]+);/gi, (match, body: string) => {
    if (body[0] === '#') {
      const isHex = body[1] === 'x' || body[1] === 'X';
      const code = parseInt(body.slice(isHex ? 2 : 1), isHex ? 16 : 10);
      if (Number.isNaN(code)) return match;
      if (code === 0xa0) return ' '; // numeric nbsp, same as the named form
      try {
        return String.fromCodePoint(code);
      } catch {
        return match;
      }
    }
    const name = body.toLowerCase();
    return name in NAMED_ENTITIES ? NAMED_ENTITIES[name] : match;
  });
}

/** Case-insensitive character compare, ASCII only — the tag and attribute
 *  names this module looks for are all ASCII, and `toLowerCase()` on a whole
 *  string cannot be used for scanning: for some characters it changes the
 *  string's *length*, so indices taken from a lowercased copy would not point
 *  at the same places in the original. */
const eq = (a: string, b: string) => a.toLowerCase() === b;

/**
 * The `href` an `<a>` tag carries, if it is one this module may follow.
 *
 * **The second and last place a URL becomes an href**, and so the second
 * place the scheme check lives — see `URL_RE`. The check is the same and for
 * the same reason: `javascript:` and `data:` never match, and an attribute
 * that is not an http(s) URL yields nothing rather than something odd.
 *
 * Entities are decoded first because a real query string arrives written
 * `?a=1&amp;b=2`, and the href is what the browser is handed, not what the
 * page displays. Quoted with either quote, or unquoted to the first
 * whitespace — the three shapes a calendar invitation actually contains.
 */
function hrefOf(tagBody: string): string | null {
  for (let i = 0; i < tagBody.length - 4; i++) {
    if (!eq(tagBody.slice(i, i + 4), 'href')) continue;
    // A bare `href` and not the tail of another name (`data-href`).
    if (i > 0 && /[\w-]/.test(tagBody[i - 1])) continue;
    let j = i + 4;
    while (j < tagBody.length && /\s/.test(tagBody[j])) j++;
    if (tagBody[j] !== '=') continue;
    j++;
    while (j < tagBody.length && /\s/.test(tagBody[j])) j++;
    const quote = tagBody[j];
    let raw: string;
    if (quote === '"' || quote === "'") {
      const end = tagBody.indexOf(quote, j + 1);
      if (end === -1) return null;
      raw = tagBody.slice(j + 1, end);
    } else {
      let end = j;
      while (end < tagBody.length && !/\s/.test(tagBody[end])) end++;
      raw = tagBody.slice(j, end);
    }
    const url = decodeEntities(raw).trim();
    // Anchored, unlike `URL_RE`: an attribute either *is* a URL we may follow
    // or is not one, and `href="see https://x"` is not a destination.
    return /^https?:\/\/[^\s<>"']+$/i.test(url) ? url : null;
  }
  return null;
}

/**
 * Splits a description into the anchors it contains and the text between
 * them, so each half can be treated as what it is.
 *
 * Anchors have to be found *before* [`stripTags`] runs, because stripping is
 * exactly what destroys the destination. Everything else stays on the old
 * path untouched.
 *
 * Linear, by the same rule the rest of this file is: every branch advances
 * `i` past what it has consumed, and nothing rescans. An `<a` with no `>`,
 * or with no `</a>` after it, is not an anchor — it falls into the text run
 * and is stripped like any other malformed tag, which is the outcome that
 * was already correct before links carried labels.
 *
 * Finding a tag's end ignores quoting, exactly as [`stripTags`] always has,
 * so `href="…<b>…"` ends the tag early and the rest becomes text. That is a
 * limitation in one direction only: [`hrefOf`] follows an href solely when
 * the whole attribute matches an anchored http(s) pattern that excludes
 * `<`, `>` and both quote characters, so a truncated attribute produces no
 * link rather than an unintended one. A quote-tracking scanner would be a
 * second HTML parser living in the module whose entire promise is that it
 * does not parse HTML.
 */
type Chunk = { kind: 'text'; raw: string } | { kind: 'anchor'; href: string | null; inner: string };

function splitAnchors(input: string): Chunk[] {
  const chunks: Chunk[] = [];
  let run = '';
  let i = 0;
  while (i < input.length) {
    const lt = input.indexOf('<', i);
    if (lt === -1) {
      run += input.slice(i);
      break;
    }
    const isAnchor =
      eq(input[lt + 1] ?? '', 'a') && /[\s>]/.test(input[lt + 2] ?? '');
    if (!isAnchor) {
      run += input.slice(i, lt + 1);
      i = lt + 1;
      continue;
    }
    const tagEnd = input.indexOf('>', lt);
    const close = tagEnd === -1 ? -1 : closingAnchor(input, tagEnd + 1);
    if (tagEnd === -1 || close === -1) {
      run += input.slice(i, lt + 1);
      i = lt + 1;
      continue;
    }
    run += input.slice(i, lt);
    if (run) chunks.push({ kind: 'text', raw: run });
    run = '';
    chunks.push({
      kind: 'anchor',
      href: hrefOf(input.slice(lt + 2, tagEnd)),
      inner: input.slice(tagEnd + 1, close),
    });
    i = input.indexOf('>', close);
    i = i === -1 ? input.length : i + 1;
  }
  if (run) chunks.push({ kind: 'text', raw: run });
  return chunks;
}

/** Index of the `<` that opens the next `</a`, or -1, which it also is when
 *  another `<a` opens first: an anchor somebody forgot to close is not a
 *  link around everything up to the next anchor's `</a>`. */
function closingAnchor(input: string, from: number): number {
  let i = from;
  while (i < input.length) {
    const lt = input.indexOf('<', i);
    if (lt === -1) return -1;
    if (input[lt + 1] === '/' && eq(input[lt + 2] ?? '', 'a') && /[\s>]/.test(input[lt + 3] ?? '')) {
      return lt;
    }
    if (eq(input[lt + 1] ?? '', 'a') && /[\s>]/.test(input[lt + 2] ?? '')) return -1;
    i = lt + 1;
  }
  return -1;
}

/** The old pipeline, minus linkifying: markup in, display text out. */
function plainText(raw: string): string {
  return decodeEntities(stripTags(raw.replace(ITEM_RE, '• ').replace(BREAK_RE, '\n'))).replace(BIDI_RE, '');
}

/**
 * Turn a raw event description into segments safe to render as text.
 *
 * Order is deliberate and load-bearing: pull out the anchors, then for
 * everything else convert line-structuring tags to newlines, strip all
 * remaining tags, decode entities, and linkify. Each step only sees output
 * that is already safe with respect to the step before it — see the comments
 * on stripTags and decodeEntities.
 *
 * The anchor pass is the only addition, and it takes nothing away: an anchor
 * whose text is its own URL (169 of the 217 on the author's real calendar)
 * comes out exactly as it always did, because label and destination are then
 * the same string. What changes is the anchor written with words, which used
 * to lose its destination in the stripper with nothing left to say so.
 */
export function descriptionSegments(raw: string | null): Segment[] {
  let input = (raw ?? '').trim();
  if (!input) return [];
  if (input.length > MAX_DESCRIPTION_LENGTH) {
    input = input.slice(0, MAX_DESCRIPTION_LENGTH);
  }

  const segments: Segment[] = [];
  for (const chunk of splitAnchors(input)) {
    if (chunk.kind === 'anchor') {
      const label = plainText(chunk.inner).trim();
      if (chunk.href && label) {
        segments.push({ kind: 'link', value: label, href: chunk.href });
      } else if (label) {
        // No usable destination — a `javascript:` href, a missing one, an
        // anchor around an image. The words stay, exactly as before; they
        // just do not become a link. Pushed through `linkify` because the
        // label may still contain a bare URL of its own.
        segments.push(...linkify(label));
      }
      continue;
    }
    segments.push(...linkify(collapse(plainText(chunk.raw))));
  }

  // Trim the ends of the whole run, which no single chunk can do for itself.
  const first = segments[0];
  if (first?.kind === 'text') first.value = first.value.replace(/^\s+/, '');
  const last = segments[segments.length - 1];
  if (last?.kind === 'text') last.value = last.value.replace(/\s+$/, '');
  return segments.filter((s) => s.kind === 'link' || s.value !== '');
}

/** Three or more newlines read as a gap, not as a stack of empty lines. */
const collapse = (text: string) => text.replace(/\n{3,}/g, '\n\n');

/** Bare URLs in already-safe text become their own link segments. */
function linkify(text: string): Segment[] {
  if (!text) return [];
  const out: Segment[] = [];
  let lastIndex = 0;
  URL_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = URL_RE.exec(text))) {
    if (match.index > lastIndex) {
      out.push({ kind: 'text', value: text.slice(lastIndex, match.index) });
    }
    const url = withoutTrailingPunctuation(match[0]);
    out.push({ kind: 'link', value: url, href: url });
    lastIndex = match.index + url.length;
    URL_RE.lastIndex = lastIndex;
  }
  if (lastIndex < text.length) {
    out.push({ kind: 'text', value: text.slice(lastIndex) });
  }
  return out;
}

/** A URL as matched, minus what is the sentence's rather than the link's: a
 *  closing full stop or comma, or a `)` with no `(` of its own in the URL
 *  (the parenthesis around a link, where Wikipedia's `Function_(mathematics)`
 *  keeps its own). */
function withoutTrailingPunctuation(url: string): string {
  let end = url.length;
  for (;;) {
    const c = url[end - 1];
    if ('.,;:!?\'"'.includes(c)) { end--; continue; }
    if (c === ')') {
      const body = url.slice(0, end);
      const opens = body.split('(').length - 1;
      const closes = body.split(')').length - 1;
      if (closes > opens) { end--; continue; }
    }
    break;
  }
  return url.slice(0, end);
}
