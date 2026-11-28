// The dictionary list: add, enable/disable, reorder, rename and remove.
// Shown in the article area instead of an article.

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
  // A failed add or change, shown until dismissed or the next action.
  const [notice, setNotice] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  // Changes run one at a time, so rapid clicks can't interleave.
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let current = true;
    api
      .dictionaries()
      .then((list) => current && setDictionaries(list))
      .catch((e) => current && setNotice(strings.loadDictionariesFailed(String(e))));
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
      setNotice(strings.libraryChangeFailed(String(e)));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const addFile = async () => {
    setAdding(true);
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: strings.dictionaryFilter, extensions: ["mdx"] }],
      });
      if (path === null) return;
      setNotice(null);
      const info = await api.addFile(path);
      if (info.status === "error") setNotice(strings.dictionaryUnusable(info.title, info.error ?? ""));
      onChanged();
    } catch (e) {
      setNotice(strings.addFailed(String(e)));
    } finally {
      setAdding(false);
    }
  };

  const count = dictionaries?.length ?? 0;
  return (
    <section className="library">
      <header className="library-header">
        <h1 className="library-title">{strings.dictionaries}</h1>
        <button className="button" onClick={addFile} disabled={adding || busy}>
          {adding ? strings.adding : strings.addFile}
        </button>
        <button className="button" onClick={onClose}>
          {strings.closeDictionaries}
        </button>
      </header>
      {notice !== null && (
        <div className="notice" role="alert">
          <span>{notice}</span>
          <button className="notice-dismiss" onClick={() => setNotice(null)} aria-label={strings.dismiss}>
            ×
          </button>
        </div>
      )}
      {dictionaries !== null && count === 0 && <p className="placeholder">{strings.emptyLibrary}</p>}
      <ol className="library-list">
        {dictionaries?.map((d, index) => (
          <DictionaryRow
            key={d.id}
            dictionary={d}
            index={index}
            last={index === count - 1}
            busy={busy || adding}
            onEnabled={(enabled) => change(() => api.setEnabled(d.id, enabled))}
            onMove={(position) => change(() => api.moveDictionary(d.id, position))}
            onRename={(name) => change(() => api.renameDictionary(d.id, name))}
            onRemove={() => change(() => api.removeDictionary(d.id))}
          />
        ))}
      </ol>
    </section>
  );
}

interface RowProps {
  dictionary: api.DictionaryInfo;
  /** Place in the list (0 is first). */
  index: number;
  last: boolean;
  busy: boolean;
  onEnabled(enabled: boolean): void;
  onMove(position: number): void;
  /** Resolves to whether the rename succeeded. */
  onRename(name: string): Promise<boolean>;
  onRemove(): void;
}

function DictionaryRow({ dictionary: d, index, last, busy, onEnabled, onMove, onRename, onRemove }: RowProps) {
  // The name being typed, while renaming.
  const [draft, setDraft] = useState<string | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  const submitRename = (e: FormEvent) => {
    e.preventDefault();
    if (draft === null) return;
    void onRename(draft).then((ok) => ok && setDraft(null));
  };

  const status = d.status === "error" ? strings.statusError(d.error ?? "") : strings.statusReady(d.headwordCount);

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
