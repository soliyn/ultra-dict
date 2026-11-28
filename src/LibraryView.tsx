// The dictionary list: add files and folders, rescan folders, and enable/
// disable, reorder, rename and remove dictionaries. Shown in the article
// area instead of an article.

import { FormEvent, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "./api";
import { strings } from "./strings";

interface Props {
  /** Bumped when the library changes, so the list is fetched again. */
  libraryVersion: number;
  /** The library changed here: search results and articles are stale. */
  onChanged(): void;
  onClose(): void;
}

export function LibraryView({ libraryVersion, onChanged, onClose }: Props) {
  const [dictionaries, setDictionaries] = useState<api.DictionaryInfo[] | null>(null);
  const [folders, setFolders] = useState<api.FolderInfo[]>([]);
  // The outcome of the last add or change (a failure, or what a folder scan
  // found), shown until dismissed or the next action.
  const [notice, setNotice] = useState<Notice | null>(null);
  const failed = (message: string) => setNotice({ message, error: true });
  // The add or scan under way: "file", "folder", or the rescanned folder's id.
  const [adding, setAdding] = useState<"file" | "folder" | number | null>(null);
  // Changes run one at a time, so rapid clicks can't interleave.
  const [busy, setBusy] = useState(false);
  // How far the indexer has got, by dictionary id, for those it is on.
  const [progress, setProgress] = useState<ReadonlyMap<number, api.IndexingEvent>>(new Map());

  useEffect(() => {
    const unlisten = api.onIndexing((event) =>
      setProgress((prev) => {
        const next = new Map(prev);
        if (event.finished) next.delete(event.dictionaryId);
        else next.set(event.dictionaryId, event);
        return next;
      }),
    );
    return () => void unlisten.then((stop) => stop());
  }, []);

  useEffect(() => {
    let current = true;
    Promise.all([api.dictionaries(), api.folders()])
      .then(([list, folderList]) => {
        if (!current) return;
        setDictionaries(list);
        setFolders(folderList);
      })
      .catch((e) => current && failed(strings.loadDictionariesFailed(String(e))));
    return () => {
      current = false;
    };
  }, [libraryVersion]);

  /** Runs one library change; its result replaces the listing. False if it failed. */
  const change = async (call: () => Promise<api.DictionaryInfo | api.DictionaryInfo[]>) => {
    setBusy(true);
    setNotice(null);
    try {
      const result = await call();
      setDictionaries((prev) =>
        Array.isArray(result) ? result : (prev ?? []).map((d) => (d.id === result.id ? result : d)),
      );
      onChanged();
      return true;
    } catch (e) {
      failed(strings.libraryChangeFailed(String(e)));
      return false;
    } finally {
      setBusy(false);
    }
  };

  /**
   * Runs one add or scan. `run` resolves to the notice to show (null for
   * none), or to `undefined` if the user cancelled the picker.
   */
  const add = async (what: "file" | "folder" | number, run: () => Promise<Notice | null | undefined>) => {
    setAdding(what);
    let changed = false;
    try {
      const outcome = await run();
      if (outcome === undefined) return;
      changed = true;
      setNotice(outcome);
    } catch (e) {
      // A scan may have added some dictionaries before it failed.
      changed = true;
      failed(strings.addFailed(String(e)));
    } finally {
      setAdding(null);
      if (changed) onChanged();
    }
  };

  const addFile = () =>
    add("file", async () => {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: strings.dictionaryFilter, extensions: ["mdx"] }],
      });
      if (path === null) return undefined;
      await api.addFile(path);
      return null;
    });

  const addFolder = () =>
    add("folder", async () => {
      const path = await open({ multiple: false, directory: true });
      if (path === null) return undefined;
      return scanNotice(await api.addFolder(path));
    });

  const rescan = (folder: api.FolderInfo) =>
    add(folder.id, async () => scanNotice(await api.rescanFolder(folder.id)));

  // Forgetting a folder changes no dictionary, so nothing else refetches.
  const forget = async (folder: api.FolderInfo) => {
    setBusy(true);
    setNotice(null);
    try {
      setFolders(await api.removeFolder(folder.id));
    } catch (e) {
      failed(strings.libraryChangeFailed(String(e)));
    } finally {
      setBusy(false);
    }
  };

  const count = dictionaries?.length ?? 0;
  const locked = adding !== null || busy;
  return (
    <section className="library">
      <header className="library-header">
        <h1 className="library-title">{strings.dictionaries}</h1>
        <button className="button" onClick={addFile} disabled={locked}>
          {adding === "file" ? strings.adding : strings.addFile}
        </button>
        <button className="button" onClick={addFolder} disabled={locked}>
          {adding === "folder" ? strings.scanning : strings.addFolder}
        </button>
        <button className="button" onClick={onClose}>
          {strings.closeDictionaries}
        </button>
      </header>
      {notice !== null && (
        <div className={"notice" + (notice.error ? "" : " info")} role={notice.error ? "alert" : "status"}>
          <span>{notice.message}</span>
          <button className="notice-dismiss" onClick={() => setNotice(null)} aria-label={strings.dismiss}>
            ×
          </button>
        </div>
      )}
      {folders.length > 0 && (
        <section className="library-folders">
          <h2 className="library-subtitle">{strings.folders}</h2>
          <ul className="library-list">
            {folders.map((folder) => (
              <li key={folder.id} className="library-row">
                <span className="library-folder-path" title={folder.path}>
                  {folder.path}
                </span>
                <div className="library-actions">
                  <button className="button" onClick={() => rescan(folder)} disabled={locked} title={strings.rescanHint}>
                    {adding === folder.id ? strings.scanning : strings.rescan}
                  </button>
                  <button
                    className="button"
                    onClick={() => forget(folder)}
                    disabled={locked}
                    title={strings.forgetFolderHint}
                  >
                    {strings.forgetFolder}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {folders.length > 0 && <h2 className="library-subtitle">{strings.dictionaries}</h2>}
      {dictionaries !== null && count === 0 && <p className="placeholder">{strings.emptyLibrary}</p>}
      <ol className="library-list">
        {dictionaries?.map((d, index) => (
          <DictionaryRow
            key={d.id}
            dictionary={d}
            index={index}
            last={index === count - 1}
            busy={locked}
            progress={progress.get(d.id)}
            onEnabled={(enabled) => change(() => api.setEnabled(d.id, enabled))}
            onReindex={() => change(() => api.reindexDictionary(d.id))}
            onStopIndexing={() => change(() => api.cancelIndexing(d.id))}
            onMove={(position) => change(() => api.moveDictionary(d.id, position))}
            onRename={(name) => change(() => api.renameDictionary(d.id, name))}
            onRemove={() => change(() => api.removeDictionary(d.id))}
          />
        ))}
      </ol>
    </section>
  );
}

interface Notice {
  message: string;
  /** Shown as an alert; otherwise as plain information. */
  error: boolean;
}

/** What a folder add or rescan found, as a notice. */
function scanNotice({ folder, added }: api.FolderScan): Notice {
  if (added.length === 0) return { message: strings.scanFoundNothing(folder.path), error: false };
  return { message: strings.scanAdded(added.length), error: false };
}

interface RowProps {
  dictionary: api.DictionaryInfo;
  /** Place in the list (0 is first). */
  index: number;
  last: boolean;
  busy: boolean;
  /** The indexer's latest progress, while it is on this dictionary. */
  progress: api.IndexingEvent | undefined;
  onEnabled(enabled: boolean): void;
  onReindex(): void;
  onStopIndexing(): void;
  onMove(position: number): void;
  /** Resolves to whether the rename succeeded. */
  onRename(name: string): Promise<boolean>;
  onRemove(): void;
}

function DictionaryRow(props: RowProps) {
  const { dictionary: d, index, last, busy, progress, onEnabled, onReindex, onStopIndexing } = props;
  const { onMove, onRename, onRemove } = props;
  // The name being typed, while renaming.
  const [draft, setDraft] = useState<string | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  const submitRename = (e: FormEvent) => {
    e.preventDefault();
    if (draft === null) return;
    void onRename(draft).then((ok) => ok && setDraft(null));
  };

  const status = statusText(d, progress);

  return (
    <li className={"library-row" + (d.enabled ? "" : " disabled")}>
      <input
        type="checkbox"
        className="library-enabled"
        checked={d.enabled}
        disabled={busy}
        onChange={(e) => onEnabled(e.target.checked)}
        aria-label={strings.enabledLabel(d.title)}
      />
      <div className="library-info">
        {draft === null ? (
          <div className="library-name" title={d.title !== d.fileTitle ? strings.originalTitle(d.fileTitle) : d.path}>
            {d.title}
          </div>
        ) : (
          <form className="library-rename" onSubmit={submitRename}>
            <input
              className="input"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && setDraft(null)}
              placeholder={d.fileTitle}
              aria-label={strings.renameLabel}
              title={strings.renameHint}
              autoFocus
              spellCheck={false}
            />
            <button className="button" type="submit" disabled={busy}>
              {strings.save}
            </button>
            <button className="button" type="button" onClick={() => setDraft(null)}>
              {strings.cancel}
            </button>
          </form>
        )}
        <div className={"library-status" + (d.status === "error" ? " error" : "")}>{d.enabled ? status : strings.statusDisabled(status)}</div>
      </div>
      <div className="library-actions">
        {d.status === "indexing" && (
          <button className="button" onClick={onStopIndexing} disabled={busy} title={strings.stopIndexingHint}>
            {strings.stopIndexing}
          </button>
        )}
        {(d.status === "needs_reindex" || d.status === "error") && (
          <button className="button" onClick={onReindex} disabled={busy} title={strings.reindexHint}>
            {strings.reindex}
          </button>
        )}
        <button
          className="button icon"
          onClick={() => onMove(index - 1)}
          disabled={busy || index === 0}
          aria-label={strings.moveUp}
          title={strings.moveUp}
        >
          ↑
        </button>
        <button
          className="button icon"
          onClick={() => onMove(index + 1)}
          disabled={busy || last}
          aria-label={strings.moveDown}
          title={strings.moveDown}
        >
          ↓
        </button>
        <button
          className="button"
          onClick={() => setDraft(d.title === d.fileTitle ? "" : d.title)}
          disabled={draft !== null}
        >
          {strings.rename}
        </button>
        {confirmingRemove ? (
          <>
            <button className="button danger" onClick={onRemove} disabled={busy} title={strings.removeHint} autoFocus>
              {strings.confirmRemove}
            </button>
            <button className="button" onClick={() => setConfirmingRemove(false)}>
              {strings.cancel}
            </button>
          </>
        ) : (
          <button className="button" onClick={() => setConfirmingRemove(true)}>
            {strings.remove}
          </button>
        )}
      </div>
    </li>
  );
}

function statusText(d: api.DictionaryInfo, progress: api.IndexingEvent | undefined): string {
  switch (d.status) {
    case "indexing":
      return progress ? strings.statusIndexing(progress.done, progress.total) : strings.statusWaiting;
    case "ready":
      return strings.statusReady(d.headwordCount);
    case "needs_reindex":
      return strings.statusNeedsReindex;
    case "error":
      return strings.statusError(d.error ?? "");
  }
}
