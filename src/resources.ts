// Dictionary resources (CSS, images, fonts, …) served by the Rust `dict` URI
// scheme (src-tauri/src/protocol.rs), and the rewriting that points article
// HTML at them.

import { convertFileSrc } from "@tauri-apps/api/core";

// `dict://localhost/` on macOS/Linux, `http://dict.localhost/` on Windows.
const BASE = convertFileSrc("", "dict");

/** The URL serving `path` (as an article writes it) from dictionary `dictId`. */
export function resourceUrl(dictId: number, path: string): string {
  const [, rawPath, suffix] = /^([^?#]*)(.*)$/s.exec(path)!;
  const segments = rawPath
    .split(/[/\\]/)
    .filter((s) => s !== "")
    .map((s) => encodeURIComponent(decodeSegment(s)));
  return `${BASE}${dictId}/${segments.join("/")}${suffix}`;
}

function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment; // a stray `%` is part of the name
  }
}

// A URL with a scheme (`http:`, `data:`, `entry:`, `sound:`, …), a
// protocol-relative one (`//host/…`), or a fragment: none is a resource path.
const NOT_A_PATH = /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i;

/**
 * Where a reference in dictionary `dictId`'s HTML or CSS should point.
 * Relative and root-relative paths, and `file://` ones, become resource
 * URLs; everything else is returned unchanged.
 */
export function resolveReference(dictId: number, ref: string): string {
  const trimmed = ref.trim();
  const file = /^file:\/\//i.exec(trimmed);
  if (file) return resourceUrl(dictId, trimmed.slice(file[0].length));
  if (trimmed === "" || NOT_A_PATH.test(trimmed)) return ref;
  return resourceUrl(dictId, trimmed);
}

// Attributes holding a resource (not a link) on any element.
const RESOURCE_ATTRIBUTES = ["src", "background", "poster"];
const CSS_URL = /url\(\s*(['"]?)(.*?)\1\s*\)/gis;
const CSS_IMPORT = /@import\s+(['"])(.*?)\1/gi;

function resolveCss(dictId: number, css: string): string {
  return css
    .replace(CSS_URL, (_, quote: string, ref: string) => {
      return `url(${quote}${resolveReference(dictId, ref)}${quote})`;
    })
    .replace(CSS_IMPORT, (_, quote: string, ref: string) => {
      return `@import ${quote}${resolveReference(dictId, ref)}${quote}`;
    });
}

/**
 * `html` (one entry of dictionary `dictId`) with its resource references
 * (stylesheet links, `src`, `background` and `poster` attributes, CSS
 * `url()` and `@import` in `style` elements and attributes) pointing at the
 * `dict` scheme. Links between entries (`<a href>`) are left alone.
 *
 * Parsing uses an inert `<template>`, so nothing loads or runs here.
 */
export function resolveResources(dictId: number, html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  for (const element of template.content.querySelectorAll("*")) {
    for (const name of RESOURCE_ATTRIBUTES) {
      const value = element.getAttribute(name);
      if (value !== null) element.setAttribute(name, resolveReference(dictId, value));
    }
    if (element.localName === "link") {
      const href = element.getAttribute("href");
      if (href !== null) element.setAttribute("href", resolveReference(dictId, href));
    }
    const style = element.getAttribute("style");
    if (style !== null) element.setAttribute("style", resolveCss(dictId, style));
    if (element.localName === "style") {
      element.textContent = resolveCss(dictId, element.textContent ?? "");
    }
  }
  return template.innerHTML;
}
