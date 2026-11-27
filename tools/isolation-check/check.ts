// Script-isolation check for "Allow scripts" (issue #12), run against the
// real app in its real webview. See README.md in this folder.
//
// Drives the running app over the Chrome DevTools Protocol: adds the
// `scripts` fixture, opens its `probe` article with scripts off and then on,
// and reports what ran and what the probe could reach. Finally it looks in
// the library database for the probe's `add_file` markers: a marker there
// means a dictionary script reached the backend.
//
//   bun tools/isolation-check/check.ts <library.sqlite3> [devtools port]

import { Database } from "bun:sqlite";
import { resolve } from "node:path";

const [dbPath, portArg] = process.argv.slice(2);
if (!dbPath) {
  console.error("usage: bun tools/isolation-check/check.ts <library.sqlite3> [devtools port]");
  process.exit(2);
}
const port = Number(portArg ?? 9223);
const fixture = resolve(import.meta.dir, "../../src-tauri/tests/fixtures/scripts/scripts.mdx");

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// --- a minimal CDP client ---------------------------------------------------

interface Target {
  type: string;
  url: string;
  webSocketDebuggerUrl: string;
}

const targets: Target[] = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((t) => t.type === "page" && !t.url.startsWith("devtools:"));
if (!page) throw new Error(`no page target on port ${port}: ${JSON.stringify(targets)}`);

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => {
  ws.onopen = r;
  ws.onerror = j;
});

let nextId = 1;
const pending = new Map<number, (msg: any) => void>();
// Execution contexts, by session: "" is the page; a frame running in its own
// process is a separate attached target with its own session.
interface Context {
  id: number;
  session: string;
  origin: string;
  frameId: string;
  isDefault: boolean;
}
const contexts = new Map<string, Context>();
const key = (session: string, id: number) => `${session}/${id}`;
ws.onmessage = (event) => {
  const msg = JSON.parse(String(event.data));
  const session: string = msg.sessionId ?? "";
  if (msg.id !== undefined) pending.get(msg.id)?.(msg);
  else if (msg.method === "Runtime.executionContextCreated") {
    const c = msg.params.context;
    contexts.set(key(session, c.id), {
      id: c.id,
      session,
      origin: c.origin,
      frameId: c.auxData?.frameId,
      isDefault: c.auxData?.isDefault,
    });
  } else if (msg.method === "Runtime.executionContextDestroyed") {
    contexts.delete(key(session, msg.params.executionContextId));
  } else if (msg.method === "Runtime.executionContextsCleared") {
    for (const [k, c] of contexts) if (c.session === session) contexts.delete(k);
  } else if (msg.method === "Target.attachedToTarget") {
    const child: string = msg.params.sessionId;
    send("Runtime.enable", {}, child).catch(() => {});
    send("Runtime.runIfWaitingForDebugger", {}, child).catch(() => {});
  }
};

function send(method: string, params: object = {}, sessionId = ""): Promise<any> {
  const id = nextId++;
  ws.send(JSON.stringify(sessionId ? { id, method, params, sessionId } : { id, method, params }));
  return new Promise((resolve, reject) =>
    pending.set(id, (msg) => (msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result))),
  );
}

async function evaluate(expression: string, contextId?: number, session = ""): Promise<any> {
  const result = await send(
    "Runtime.evaluate",
    { expression, contextId, awaitPromise: true, returnByValue: true },
    session,
  );
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}

async function until<T>(what: string, probe: () => Promise<T | null | undefined>, ms = 10_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const value = await probe().catch(() => null);
    if (value !== null && value !== undefined) return value;
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what}`);
}

// --- the check ----------------------------------------------------------------

await send("Runtime.enable");
await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
const { frameTree } = await send("Page.getFrameTree");
const mainFrameId: string = frameTree.frame.id;
const mainContext = () =>
  [...contexts.values()].find((c) => c.session === "" && c.frameId === mainFrameId && c.isDefault)?.id;
await until("main context", async () => mainContext());

const say = (line: string) => console.log(line);

say(`app: ${page.url}`);
const info = await evaluate(
  `window.__TAURI_INTERNALS__.invoke("add_file", { path: ${JSON.stringify(fixture)} })`,
  mainContext(),
);
say(`added: ${info.title} (id ${info.id}, ${info.status}, allowScripts ${info.allowScripts})`);
if (info.allowScripts) {
  await evaluate(
    `window.__TAURI_INTERNALS__.invoke("set_allow_scripts", { dictionaryId: ${info.id}, allow: false })`,
    mainContext(),
  );
}

// Start from a fresh page, so no article from an earlier run is open.
await send("Page.reload");
await sleep(500);
await until("the reloaded page", () =>
  evaluate(`document.querySelector("input.search") ? true : null`, mainContext()),
);

// Search for the probe headword and open it.
await evaluate(
  `(() => {
    const input = document.querySelector("input.search");
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, "probe");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  })()`,
  mainContext(),
);
await until("the probe result row", () =>
  evaluate(
    `[...document.querySelectorAll(".result-row")].find((r) => r.textContent === "probe")?.click() ?? null`,
    mainContext(),
  ).then(() =>
    evaluate(`document.querySelector(".article-headword")?.textContent === "probe" || null`, mainContext()),
  ),
);

// Phase 1: scripts off.
const off = await until("the scripts-off frame", () =>
  evaluate(
    `(() => {
      const frame = document.querySelector("iframe.entry-frame");
      const doc = frame?.contentDocument;
      const li = doc?.getElementById("inline");
      if (!li || doc.readyState !== "complete") return null;
      return {
        sandbox: frame.getAttribute("sandbox"),
        marks: ["inline", "external", "loose"].map((id) => doc.getElementById(id).textContent),
        report: doc.getElementById("report").textContent,
      };
    })()`,
    mainContext(),
  ),
);
say("");
say(`scripts off: sandbox="${off.sandbox}"`);
for (const mark of off.marks) say(`  ${mark}`);
say(`  probe report: ${off.report === "" ? "(empty: probe.js never ran)" : off.report}`);

// Phase 2: scripts on, through the checkbox.
await evaluate(`document.querySelector(".allow-scripts input").click()`, mainContext());
const on = await until("the scripted frame", () =>
  evaluate(
    `(() => {
      const frame = document.querySelector("iframe.entry-frame");
      if (frame?.getAttribute("sandbox") !== "allow-scripts") return null;
      let parentCanRead;
      try { parentCanRead = frame.contentDocument !== null; } catch { parentCanRead = false; }
      return { sandbox: frame.getAttribute("sandbox"), parentCanRead, height: frame.style.height };
    })()`,
    mainContext(),
  ),
);
// The probe runs its last attempt after 1.5 s and gives each 5 s to answer.
await sleep(8000);
const inside = await until("the probe report", async () => {
  for (const c of contexts.values()) {
    if (c.frameId === mainFrameId || !c.isDefault) continue;
    const value = await evaluate(
      `document.getElementById("report") && {
        origin: window.origin,
        marks: ["inline", "external", "loose"].map((id) => document.getElementById(id).textContent),
        report: document.getElementById("report").textContent,
      }`,
      c.id,
      c.session,
    ).catch(() => null);
    if (value) return value;
  }
  return null;
});
say("");
say(`scripts on: sandbox="${on.sandbox}", frame origin ${JSON.stringify(inside.origin)}`);
say(`  parent can read the frame document: ${on.parentCanRead}`);
say(`  frame height from postMessage: ${on.height}`);
for (const mark of inside.marks) say(`  ${mark}`);
say("  probe report:");
for (const line of String(inside.report).split("\n")) say(`    ${line}`);

ws.close();

// The ground truth: did any route reach the backend?
await sleep(1000);
const db = new Database(dbPath, { readonly: true });
const markers = db
  .query("SELECT path, status, error FROM dictionaries WHERE path LIKE '%ultra-dict-ipc-probe-%'")
  .all();
db.close();
say("");
say(
  markers.length === 0
    ? "backend: no probe marker in the library: no dictionary script reached add_file"
    : `backend: REACHED by ${JSON.stringify(markers)}`,
);

const ok =
  markers.length === 0 &&
  off.sandbox === "allow-same-origin" &&
  off.marks.every((m: string) => m.endsWith("did not run")) &&
  off.report === "" &&
  on.sandbox === "allow-scripts" &&
  !on.parentCanRead &&
  inside.marks.every((m: string) => m.endsWith(": ran"));
say(
  ok
    ? "RESULT: isolated"
    : "RESULT: NOT ISOLATED or check incomplete; read the report above. If probe.js and " +
        "loose.js did not run with scripts on, the app was likely started without " +
        "IsolateSandboxedIframes disabled (see README.md).",
);
process.exit(ok ? 0 : 1);
