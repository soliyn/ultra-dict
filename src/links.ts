// Links between entries (docs/spec-v1.md, "Links"): `entry://word`, and the
// `bword://word` some dictionaries write instead, look `word` up in the app.
// `entry://#name` points at an anchor in the same article.

const ENTRY_LINK = /^\s*(?:entry|bword):\/\/(.*)$/is;

export type EntryLink = { word: string } | { anchor: string };

/** What an entry link points at, or null for any other link (or an empty one). */
export function entryLink(href: string): EntryLink | null {
  const match = ENTRY_LINK.exec(href);
  if (match === null) return null;
  const rest = match[1].trim();
  if (rest.startsWith("#")) {
    const anchor = decode(rest.slice(1));
    return anchor === "" ? null : { anchor };
  }
  // `entry://word#section` opens `word`; the section is dropped.
  const word = decode(rest.replace(/#.*$/s, "")).trim();
  return word === "" ? null : { word };
}

/** `text` with its `%XX` escapes decoded, as far as they are valid. */
function decode(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text; // a stray `%` is part of the word
  }
}

/** The element an `entry://#name` link means in `doc`, by id, then by name. */
export function anchorTarget(doc: Document, anchor: string): Element | null {
  return doc.getElementById(anchor) ?? doc.getElementsByName(anchor)[0] ?? null;
}
