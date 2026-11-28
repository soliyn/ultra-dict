-- Library database, schema version 4 (PRAGMA user_version).
--
-- This file creates the latest schema on a new database. Upgrades of
-- existing ones are the MIGRATIONS in mod.rs: change both together.
--
-- Dictionary groups (out of v1) will be their own tables referencing
-- dictionaries(id); nothing here assumes a single flat collection beyond
-- the global `position`.

-- One row per dictionary file the user added.
CREATE TABLE dictionaries (
    id            INTEGER PRIMARY KEY,
    -- Absolute path of the dictionary file, referenced in place; canonical
    -- (see `normalize` in mod.rs) if the file existed when it was added.
    path          TEXT    NOT NULL UNIQUE,
    -- Display title read from the file (HTML stripped, falls back to the file name).
    -- Reindexing rewrites it.
    title         TEXT    NOT NULL,
    -- The user's rename, shown instead of `title` (NULL if none). Reindexing
    -- never touches it.
    user_title    TEXT,
    -- Library order: drives article order in lookups. 0 is first.
    position      INTEGER NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    -- File size and modification time when indexed (NULL if unreadable).
    file_size     INTEGER,
    file_mtime_ns INTEGER,
    -- 'ready' or 'error'.
    status        TEXT    NOT NULL,
    -- Human-readable reason when status is 'error'.
    error         TEXT,
    -- Whether the dictionary's own JavaScript may run in its articles.
    allow_scripts INTEGER NOT NULL DEFAULT 0
);

-- Folders the user added, remembered so they can be rescanned. Dictionaries
-- found in them are ordinary rows in `dictionaries`; forgetting a folder
-- leaves them in place.
CREATE TABLE folders (
    id   INTEGER PRIMARY KEY,
    -- Canonical absolute path of the folder.
    path TEXT    NOT NULL UNIQUE
);

-- Every headword of every successfully indexed dictionary.
CREATE TABLE headwords (
    id            INTEGER PRIMARY KEY,
    dictionary_id INTEGER NOT NULL REFERENCES dictionaries (id) ON DELETE CASCADE,
    -- Exactly as stored in the dictionary.
    headword      TEXT    NOT NULL,
    -- Lowercase, diacritics stripped: what search matches against.
    folded        TEXT    NOT NULL,
    -- Format-specific opaque entry locator (EntryLocator::to_raw).
    locator       INTEGER NOT NULL
);

-- Search: prefix range on `folded`, rows already in result order, and
-- covering (dictionary_id included) so matching never touches the table.
CREATE INDEX headwords_by_folded ON headwords (folded, headword, dictionary_id);
CREATE INDEX headwords_by_dictionary ON headwords (dictionary_id);

-- Recent lookups, newest last.
CREATE TABLE history (
    id           INTEGER PRIMARY KEY,
    headword     TEXT    NOT NULL,
    -- Unix time in seconds.
    looked_up_at INTEGER NOT NULL
);
