// The article area: every dictionary's entries for one headword, each
// dictionary in its own sandboxed iframe (docs/spec-v1.md, "Rendering").

import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { resolveResources } from "./resources";
import { strings } from "./strings";

interface Props {
  headword: string;
  /** Bumped when the library changes, so the article is fetched again. */
  libraryVersion: number;
}

type Loaded = { headword: string } & ({ article: api.Article } | { error: string });

export function ArticleView({ headword, libraryVersion }: Props) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [settingFailed, setSettingFailed] = useState<string | null>(null);

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

  // The setting only changes how sections render, so update them in place
  // rather than refetching the article.
  const setAllowScripts = (dictionaryId: number, allow: boolean) => {
    setSettingFailed(null);
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
      .catch((e) => setSettingFailed(strings.settingFailed(String(e))));
  };

  // Until the new article arrives, keep showing the previous one rather than
  // flashing an empty pane.
  return (
    <article className="article">
      <h1 className="article-headword">{loaded?.headword ?? headword}</h1>
      {settingFailed !== null && <p className="article-error">{settingFailed}</p>}
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
              />
            )}
          </section>
        ))
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

// Message type a scripted frame uses to report its content height.
const HEIGHT_MESSAGE = "ultra-dict:height";

// Runs first in a scripted frame: the parent can't look into an opaque-origin
// document, so the document reports its own height.
const HEIGHT_REPORTER = `<script>(() => {
  const root = document.documentElement;
  const report = () => parent.postMessage(
    { type: "${HEIGHT_MESSAGE}", height: Math.ceil(root.getBoundingClientRect().height) }, "*");
  new ResizeObserver(report).observe(root);
  addEventListener("load", report);
})();</script>`;

function frameDocument(dictionaryId: number, entries: string[], allowScripts: boolean): string {
  const body = entries
    .map((html) => `<div class="ultra-dict-entry">${resolveResources(dictionaryId, html)}</div>`)
    .join("");
  const reporter = allowScripts ? HEIGHT_REPORTER : "";
  return `<!doctype html><html><head><meta charset="utf-8">${reporter}${FRAME_STYLE}</head><body>${body}</body></html>`;
}

interface FrameProps {
  title: string;
  dictionaryId: number;
  entries: string[];
  allowScripts: boolean;
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
function EntryFrame({ title, dictionaryId, entries, allowScripts }: FrameProps) {
  const srcDoc = useMemo(
    () => frameDocument(dictionaryId, entries, allowScripts),
    [dictionaryId, entries, allowScripts],
  );
  return allowScripts ? (
    <ScriptedFrame title={title} srcDoc={srcDoc} />
  ) : (
    <StaticFrame title={title} srcDoc={srcDoc} />
  );
}

interface DocumentFrameProps {
  title: string;
  srcDoc: string;
}

/** Scripts off: measured directly through the shared origin. */
function StaticFrame({ title, srcDoc }: DocumentFrameProps) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(0);

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
    const onLoad = () => {
      // A new article is a new document: watch its root instead.
      if (observedRoot) observer.unobserve(observedRoot);
      observedRoot = frame.contentDocument?.documentElement ?? null;
      if (observedRoot) observer.observe(observedRoot);
      measure();
    };
    frame.addEventListener("load", onLoad);
    return () => {
      observer.disconnect();
      frame.removeEventListener("load", onLoad);
    };
  }, []);

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

/** Scripts on: opaque origin; the frame reports its own height. */
function ScriptedFrame({ title, srcDoc }: DocumentFrameProps) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(0);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.source === null || event.source !== ref.current?.contentWindow) return;
      const data: unknown = event.data;
      if (typeof data !== "object" || data === null) return;
      const { type, height } = data as { type?: unknown; height?: unknown };
      if (type !== HEIGHT_MESSAGE || typeof height !== "number" || !Number.isFinite(height)) return;
      setHeight(Math.min(Math.max(Math.ceil(height), 0), MAX_FRAME_HEIGHT));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

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
