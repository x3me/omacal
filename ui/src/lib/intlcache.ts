// `Intl.DateTimeFormat` instances, built once per (locale, options) and kept.
//
// Building one costs far more than using one, and the hot paths built a fresh
// one on every call: the hour gutter's second-zone clock once per hour line,
// the zone abbreviation, and every date label. What is cached is the
// formatter, never its output, so an abbreviation that changes with DST is
// still asked of the formatter for the instant in hand.

const cache = new Map<string, Intl.DateTimeFormat>();
/** Bounded, so a session that walks many zones cannot grow it forever. */
const LIMIT = 128;

/** The formatter `new Intl.DateTimeFormat(locale, options)` would build. A
 *  zone the engine refuses throws, as the constructor does, and is not
 *  remembered. */
export function dateTimeFormat(
  locale: string | undefined,
  options: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
  const key = JSON.stringify([locale ?? null, options]);
  const hit = cache.get(key);
  if (hit) return hit;
  const made = new Intl.DateTimeFormat(locale, options);
  if (cache.size >= LIMIT) cache.clear();
  cache.set(key, made);
  return made;
}
