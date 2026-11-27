// The article area: every dictionary's entries for one headword, each
// dictionary in its own sandboxed iframe (docs/spec-v1.md, "Rendering").

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { RESOURCE_BASE, resolveResources } from "./resources";
import { playSound, soundPath } from "./sound";
import { strings } from "./strings";

interface Props {
  headword: string;
  /** Bumped when the library changes, so the article is fetched again. */
  libraryVersion: number;
}

type Loaded = { headword: string } & ({ article: api.Article } | { error: string });

export function ArticleView({ headword, libraryVersion }: Props) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [settingError, setSettingError] = useState<string | null>(null);
  // A pronunciation that couldn't play, until dismissed or the next one.
  const [soundError, setSoundError] = useState<string | null>(null);

  useEffect(() => {
    let current = true; // responses for an older headword are dropped
    api
      .article(headword)
      .then((article) => current && setLoaded({ headword, article }))
      .catch((e) => current && setLoaded({ headword, error: String(e) }));
    return () => {
      current = false;
    };
  }, [headword, libraryVersion]);

  useEffect(() => setSoundError(null), [headword]);

  const onSoundLink = useCallback((dictionaryId: number, href: string) => {
    const path = soundPath(href);
    if (path === null) return;
    setSoundError(null);
    void playSound(dictionaryId, path).then((outcome) => {
      if (outcome === "notFound") setSoundError(strings.soundNotFound(path));
      else if (outcome === "unsupported") setSoundError(strings.soundUnsupported(path));
    });
  }, []);

  // The setting only changes how sections render, so update them in place
  // rather than refetching the article.
  const setAllowScripts = (dictionaryId: number, allow: boolean) => {
    setSettingError(null);
    api
      .setAllowScripts(dictionaryId, allow)
      .then((info) =>
        setLoaded((prev) => {
          if (prev === null || "error" in prev) return prev;
          const sections = prev.article.sections.map((s) =>
            s.dictionaryId === info.id ? { ...s, allowScripts: info.allowScripts } : s,
          );
          return { ...prev, article: { sections } };
        }),
      )
      .catch((e) => setSettingError(strings.settingFailed(String(e))));
  };

  // Until the new article arrives, keep showing the previous one rather than
  // flashing an empty pane.
  return (
    <article className="article">
      <h1 className="article-headword">{loaded?.headword ?? headword}</h1>
      {settingError !== null && <p className="article-error">{settingError}</p>}
      {loaded === null ? null : "error" in loaded ? (
        <p className="article-error">{strings.articleFailed(loaded.error)}</p>
      ) : loaded.article.sections.length === 0 ? (
        <p className="placeholder">{strings.noArticle}</p>
      ) : (
        loaded.article.sections.map((section) => (
          <section className="article-section" key={section.dictionaryId}>
            <header className="article-section-header">
              <h2 className="article-dictionary">{section.dictionaryTitle}</h2>
              {section.error === null && (
                <label className="allow-scripts" title={strings.allowScriptsHint}>
                  <input
                    type="checkbox"
                    checked={section.allowScripts}
                    onChange={(e) => setAllowScripts(section.dictionaryId, e.target.checked)}
                  />
                  {strings.allowScripts}
                </label>
              )}
            </header>
            {section.error !== null ? (
              <p className="article-error">{strings.sectionFailed(section.error)}</p>
            ) : (
              <EntryFrame
                title={section.dictionaryTitle}
                dictionaryId={section.dictionaryId}
                entries={section.entries}
                allowScripts={section.allowScripts}
                onSoundLink={onSoundLink}
              />
            )}
          </section>
        ))
      )}
      {soundError !== null && (
        <div className="notice sound-notice" role="alert">
          <span>{soundError}</span>
          <button className="notice-dismiss" onClick={() => setSoundError(null)} aria-label={strings.dismiss}>
            ×
          </button>
        </div>
      )}
    </article>
  );
}

// Dictionary CSS assumes a white page; these come first so it can override them.
const FRAME_STYLE = `<style>
html { background: #fff; color: #000; color-scheme: light; overflow: hidden; }
body { margin: 8px; font-family: system-ui, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; }
.ultra-dict-entry + .ultra-dict-entry { border-top: 1px solid #ddd; margin-top: 12px; padding-top: 12px; }
</style>`;

// Message types a scripted frame posts: its content height, and a click on
// a `sound://` link.
const HEIGHT_MESSAGE = "ultra-dict:height";
const SOUND_MESSAGE = "ultra-dict:sound";

// The `sound://` link a click landed on, if any. Duck-typed: in a
// scripts-off frame the target belongs to the frame's realm, where
// `instanceof Element` fails.
function soundLinkOf(event: Event): string | null {
  const target = event.target as Partial<Element> | null;
  const anchor = typeof target?.closest === "function" ? target.closest("a[href]") : null;
  const href = anchor?.getAttribute("href") ?? null;
  return href !== null && soundPath(href) !== null ? href : null;
}

// Runs first in a scripted frame: the parent can't look into an opaque-origin
// document, so the document reports its own height and its sound clicks.
const FRAME_SCRIPT = `<script>(() => {
  const root = document.documentElement;
  const report = () => parent.postMessage(
    { type: "${HEIGHT_MESSAGE}", height: Math.ceil(root.getBoundingClientRect().height) }, "*");
  new ResizeObserver(report).observe(root);
  addEventListener("load", report);
  // Sounds play in the app window: hand it the link instead of navigating
  // the frame to a scheme it can't load. Capture phase on the window, so
  // this runs before the dictionary's own handlers.
  addEventListener("click", (event) => {
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    const href = anchor && anchor.getAttribute("href");
    if (!href || !/^\\s*sound:\\/\\//i.test(href)) return;
    event.preventDefault();
    parent.postMessage({ type: "${SOUND_MESSAGE}", href }, "*");
  }, true);
})();</script>`;

// Second layer for a scripted frame, after the opaque origin: its scripts
// come only from the `dict` scheme (or inline), and it can open connections
// to nothing else. That keeps them off Tauri's IPC endpoint and the
// internet whatever Tauri does with opaque origins. Passive content (images,
// styles, fonts, media) may still come from `https:` as in a scripts-off
// frame. A script can't lift a policy once the document has it.
const SCRIPTED_CSP = [
  "default-src 'none'",
  `script-src ${RESOURCE_BASE} 'unsafe-inline' 'unsafe-eval'`,
  `connect-src ${RESOURCE_BASE}`,
  `style-src ${RESOURCE_BASE} 'unsafe-inline' https: data:`,
  `img-src ${RESOURCE_BASE} https: data: blob:`,
  `font-src ${RESOURCE_BASE} https: data:`,
  `media-src ${RESOURCE_BASE} https: data: blob:`,
  "form-action 'none'",
  "base-uri 'none'",
].join("; ");

function frameDocument(dictionaryId: number, entries: string[], allowScripts: boolean): string {
  const body = entries
    .map((html) => `<div class="ultra-dict-entry">${resolveResources(dictionaryId, html)}</div>`)
    .join("");
  // The policy must come before anything it governs.
  const scripted = allowScripts
    ? `<meta http-equiv="Content-Security-Policy" content="${SCRIPTED_CSP}">${FRAME_SCRIPT}`
    : "";
  return `<!doctype html><html><head><meta charset="utf-8">${scripted}${FRAME_STYLE}</head><body>${body}</body></html>`;
}

interface FrameProps {
  title: string;
  dictionaryId: number;
  entries: string[];
  allowScripts: boolean;
  /** A `sound://` link in the frame was clicked. */
  onSoundLink: (dictionaryId: number, href: string) => void;
}

/**
 * One dictionary's entries in an iframe as tall as its content. Their
 * stylesheets, images and fonts load from the `dict` scheme.
 *
 * Isolation (docs/spec-v1.md, "Scripts"): a frame may run scripts or share
 * the app's origin, never both.
 * - Scripts off: `sandbox="allow-same-origin"`. Nothing inside can run; the
 *   shared origin only lets this component read the content height.
 * - Scripts on: `sandbox="allow-scripts"` alone. The frame's origin is
 *   opaque, so its scripts can't reach into this window, and Tauri refuses
 *   their IPC requests. It reports its height by `postMessage`.
 *
 * Toggling renders the other component, so the frame is recreated: a
 * sandbox change only applies to a new document anyway.
 */
function EntryFrame({ title, dictionaryId, entries, allowScripts, onSoundLink }: FrameProps) {
  const srcDoc = useMemo(
    () => frameDocument(dictionaryId, entries, allowScripts),
    [dictionaryId, entries, allowScripts],
  );
  const onSound = useCallback(
    (href: string) => onSoundLink(dictionaryId, href),
    [onSoundLink, dictionaryId],
  );
  return allowScripts ? (
    <ScriptedFrame title={title} srcDoc={srcDoc} onSound={onSound} />
  ) : (
    <StaticFrame title={title} srcDoc={srcDoc} onSound={onSound} />
  );
}

interface DocumentFrameProps {
  title: string;
  srcDoc: string;
  /** A `sound://` link in the frame was clicked. */
  onSound: (href: string) => void;
}

/** A ref holding the latest render's `value`, for long-lived listeners. */
function useLatest<T>(value: T) {
  const ref = useRef(value);
  useEffect(() => {
    ref.current = value;
  });
  return ref;
}

/**
 * Scripts off: measured directly through the shared origin, which also lets
 * this component catch clicks on `sound://` links. The listener is the
 * app's code, so it runs although the frame's own scripts can't.
 */
function StaticFrame({ title, srcDoc, onSound }: DocumentFrameProps) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(0);
  const latestOnSound = useLatest(onSound);

  useEffect(() => {
    const frame = ref.current;
    if (frame === null) return;
    const measure = () => {
      const root = frame.contentDocument?.documentElement;
      if (root) setHeight(Math.ceil(root.getBoundingClientRect().height));
    };
    // The content reflows when the pane's width changes, and grows when
    // late resources (fonts, images without a size) arrive.
    const observer = new ResizeObserver(measure);
    observer.observe(frame);
    let observedRoot: Element | null = null;
    const onClick = (event: MouseEvent) => {
      const href = soundLinkOf(event);
      if (href === null) return;
      event.preventDefault();
      latestOnSound.current(href);
    };
    const onLoad = () => {
      // A new article is a new document: watch its root and its clicks
      // instead. The old listener goes with the old document.
      if (observedRoot) observer.unobserve(observedRoot);
      observedRoot = frame.contentDocument?.documentElement ?? null;
      if (observedRoot) observer.observe(observedRoot);
      frame.contentDocument?.addEventListener("click", onClick);
      measure();
    };
    frame.addEventListener("load", onLoad);
    return () => {
      observer.disconnect();
      frame.removeEventListener("load", onLoad);
      frame.contentDocument?.removeEventListener("click", onClick);
    };
  }, [latestOnSound]);

  return (
    <iframe
      ref={ref}
      className="entry-frame"
      title={title}
      sandbox="allow-same-origin"
      srcDoc={srcDoc}
      style={{ height }}
    />
  );
}

// Cap on a reported height: the frame's own scripts can post anything.
const MAX_FRAME_HEIGHT = 200_000;

// Cap on a reported link's length, for the same reason.
const MAX_LINK_LENGTH = 4096;

/**
 * Scripts on: opaque origin; the frame reports its own height and its
 * `sound://` clicks. Only this frame's messages count, and the link can
 * only name a resource of this frame's dictionary.
 */
function ScriptedFrame({ title, srcDoc, onSound }: DocumentFrameProps) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(0);
  const latestOnSound = useLatest(onSound);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.source === null || event.source !== ref.current?.contentWindow) return;
      const data: unknown = event.data;
      if (typeof data !== "object" || data === null) return;
      const { type, height, href } = data as { type?: unknown; height?: unknown; href?: unknown };
      if (type === HEIGHT_MESSAGE && typeof height === "number" && Number.isFinite(height)) {
        setHeight(Math.min(Math.max(Math.ceil(height), 0), MAX_FRAME_HEIGHT));
      } else if (type === SOUND_MESSAGE && typeof href === "string" && href.length <= MAX_LINK_LENGTH) {
        latestOnSound.current(href);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [latestOnSound]);

  return (
    <iframe
      ref={ref}
      className="entry-frame"
      title={title}
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      style={{ height }}
    />
  );
}
