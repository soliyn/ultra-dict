// Pronunciation links (`sound://…`, docs/spec-v1.md, "Rendering"): the
// audio comes from the dictionary's resources over the `dict` scheme and
// plays in the app window, not in the article frame.

import { resourceUrl } from "./resources";

const SOUND_LINK = /^\s*sound:\/\/(.*)$/is;

/** The resource path a `sound://` link refers to, or null for any other link. */
export function soundPath(href: string): string | null {
  const match = SOUND_LINK.exec(href);
  if (match === null) return null;
  const path = match[1].trim();
  return path === "" ? null : path;
}

/**
 * How a play attempt went: started, no such resource, a format the webview
 * can't decode, or replaced by a later sound before it started.
 */
export type SoundOutcome = "playing" | "notFound" | "unsupported" | "replaced";

let current: HTMLAudioElement | null = null;

/**
 * Plays `path` from dictionary `dictId`, stopping whatever was playing.
 *
 * The webview plays mp3, wav and ogg; anything else (e.g. Speex `.spx`)
 * fails as "unsupported". A missing resource fails the same way to the
 * audio element, so only after a failure is the URL fetched to tell the two
 * apart.
 */
export async function playSound(dictId: number, path: string): Promise<SoundOutcome> {
  current?.pause();
  const url = resourceUrl(dictId, path);
  const audio = new Audio(url);
  current = audio;
  try {
    await audio.play();
    return "playing";
  } catch {
    // play() rejects with NotSupportedError for a source it can't use, and
    // with AbortError when a later sound paused this one first.
    if (current !== audio) return "replaced";
  }
  const found = await fetch(url).then(
    async (response) => response.ok && (await response.blob()).size > 0,
    () => false,
  );
  return found ? "unsupported" : "notFound";
}
