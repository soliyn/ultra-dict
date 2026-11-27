-- Library database, schema version 1 (PRAGMA user_version).
--
-- Dictionary groups (out of v1) will be their own tables referencing
-- dictionaries(id); nothing here assumes a single flat collection beyond
-- the global `position`.

-- One row per dictionary file the user added.
CREATE TABLE dictionaries (
    id            INTEGER PRIMARY KEY,
    -- Absolute path of the dictionary file, referenced in place.
    path          TEXT    NOT NULL UNIQUE,
    -- Display title read from the file (HTML stripped, falls back to the file name).
    title         TEXT    NOT NULL,
    -- Library order: drives article order in lookups. 0 is first.
    position      INTEGER NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    -- File size and modification time when indexed (NULL if unreadable).
    file_size     INTEGER,
    file_mtime_ns INTEGER,
    -- 'ready' or 'error'.
    status        TEXT    NOT NULL,
    -- Human-readable reason when status is 'error'.
    error         TEXT
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

CREATE INDEX headwords_by_folded ON headwords (folded, headword);
CREATE INDEX headwords_by_dictionary ON headwords (dictionary_id);

-- Recent lookups, newest last.
CREATE TABLE history (
    id           INTEGER PRIMARY KEY,
    headword     TEXT    NOT NULL,
    -- Unix time in seconds.
    looked_up_at INTEGER NOT NULL
);
