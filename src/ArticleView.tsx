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

  // Until the new article arrives, keep showing the previous one rather than
  // flashing an empty pane.
  return (
    <article className="article">
      <h1 className="article-headword">{loaded?.headword ?? headword}</h1>
      {loaded === null ? null : "error" in loaded ? (
        <p className="article-error">{strings.articleFailed(loaded.error)}</p>
      ) : loaded.article.sections.length === 0 ? (
        <p className="placeholder">{strings.noArticle}</p>
      ) : (
        loaded.article.sections.map((section) => (
          <section className="article-section" key={section.dictionaryId}>
            <h2 className="article-dictionary">{section.dictionaryTitle}</h2>
            {section.error !== null ? (
              <p className="article-error">{strings.sectionFailed(section.error)}</p>
            ) : (
              <EntryFrame
                title={section.dictionaryTitle}
                dictionaryId={section.dictionaryId}
                entries={section.entries}
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

function frameDocument(dictionaryId: number, entries: string[]): string {
  const body = entries
    .map((html) => `<div class="ultra-dict-entry">${resolveResources(dictionaryId, html)}</div>`)
    .join("");
  return `<!doctype html><html><head><meta charset="utf-8">${FRAME_STYLE}</head><body>${body}</body></html>`;
}

/**
 * One dictionary's entries in an iframe as tall as its content. Their
 * stylesheets, images and fonts load from the `dict` scheme.
 *
 * The sandbox omits `allow-scripts`, so nothing inside the frame can run,
 * let alone reach the Tauri IPC. It keeps `allow-same-origin` only so this
 * component can read the content height; once a dictionary may run scripts
 * (#12), its frame must drop that and get an opaque origin.
 */
function EntryFrame({
  title,
  dictionaryId,
  entries,
}: {
  title: string;
  dictionaryId: number;
  entries: string[];
}) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(0);
  const srcDoc = useMemo(() => frameDocument(dictionaryId, entries), [dictionaryId, entries]);

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
