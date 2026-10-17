//! SQLite request log (WAL). Append-only: rows are never updated.
//! Restore = nearest checkpoint + replay of stored applied results (never re-parse).

use std::path::Path;

use dsl::interp::Note;
use dsl::state::{Change, State};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Bumped when the DSL grammar changes meaning; stored per log row.
pub const GRAMMAR_VERSION: i64 = 1;

/// A checkpoint is written after every `CHECKPOINT_EVERY` entries of a workspace.
pub const CHECKPOINT_EVERY: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Claude,
    User,
}

impl Actor {
    fn as_str(self) -> &'static str {
        match self {
            Actor::Claude => "claude",
            Actor::User => "user",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "user" { Actor::User } else { Actor::Claude }
    }
}

/// One stored log row, as read back for req change status.
pub struct StoredEntry {
    pub seq: u64,
    pub actor: Actor,
    pub result: AppliedResult,
}

/// Stored applied result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedResult {
    pub changes: Vec<Change>,
    pub notes: Vec<Note>,
}

pub struct LogEntry<'a> {
    pub workspace: &'a str,
    pub seq: u64,
    pub time_ms: i64,
    pub actor: Actor,
    /// `dsl` (request source) or `op` (user operation).
    pub kind: &'a str,
    /// DSL source, or the operation in short text form.
    pub source: &'a str,
    pub result: &'a AppliedResult,
}

pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub key_hash: Option<String>,
    pub created_ms: i64,
    /// In the trash since (Unix ms).
    pub trashed_ms: Option<i64>,
}

pub struct LoadedWorkspace {
    pub row: WorkspaceRow,
    pub state: State,
    pub revision: u64,
    pub updated_ms: i64,
    pub last_notes: Vec<Note>,
}

pub struct Store {
    conn: Connection,
}

/// AST JSON nests deeper than serde_json's default limit of 128; nesting is
/// bounded by the parser (64 levels), and `serde_stacker` keeps the stack safe.
pub fn from_json<T: for<'de> Deserialize<'de>>(s: &str) -> serde_json::Result<T> {
    let mut de = serde_json::Deserializer::from_str(s);
    de.disable_recursion_limit();
    let de = serde_stacker::Deserializer::new(&mut de);
    T::deserialize(de)
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspaces (
                 id         TEXT PRIMARY KEY,
                 name       TEXT NOT NULL,
                 created_ms INTEGER NOT NULL
             );
             -- Key changes are their own append-only table; the latest row wins.
             CREATE TABLE IF NOT EXISTS workspace_keys (
                 workspace  TEXT NOT NULL REFERENCES workspaces(id),
                 at_ms      INTEGER NOT NULL,
                 key_hash   TEXT NOT NULL,
                 id         INTEGER PRIMARY KEY AUTOINCREMENT
             );
             -- Trash moves are their own append-only table; the latest row wins.
             CREATE TABLE IF NOT EXISTS workspace_trash (
                 workspace  TEXT NOT NULL REFERENCES workspaces(id),
                 at_ms      INTEGER NOT NULL,
                 trashed    INTEGER NOT NULL,
                 id         INTEGER PRIMARY KEY AUTOINCREMENT
             );
             CREATE TABLE IF NOT EXISTS log (
                 workspace        TEXT NOT NULL REFERENCES workspaces(id),
                 seq              INTEGER NOT NULL,
                 time_ms          INTEGER NOT NULL,
                 actor            TEXT NOT NULL,
                 kind             TEXT NOT NULL,
                 source           TEXT NOT NULL,
                 grammar_version  INTEGER NOT NULL,
                 result           TEXT NOT NULL,
                 PRIMARY KEY (workspace, seq)
             );
             CREATE TABLE IF NOT EXISTS checkpoints (
                 workspace  TEXT NOT NULL REFERENCES workspaces(id),
                 seq        INTEGER NOT NULL,
                 state      TEXT NOT NULL,
                 PRIMARY KEY (workspace, seq)
             );",
        )?;
        Ok(Store { conn })
    }

    pub fn create_workspace(&self, id: &str, name: &str, now_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO workspaces (id, name, created_ms) VALUES (?1, ?2, ?3)", params![id, name, now_ms])?;
        Ok(())
    }

    /// Moves a workspace to the trash (`trashed`) or back out of it.
    pub fn set_trashed(&self, workspace: &str, trashed: bool, now_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO workspace_trash (workspace, at_ms, trashed) VALUES (?1, ?2, ?3)", params![workspace, now_ms, trashed as i64])?;
        Ok(())
    }

    /// Deletes a workspace and everything recorded for it (emptying the trash).
    pub fn delete_workspace(&mut self, workspace: &str) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for table in ["log", "checkpoints", "workspace_keys", "workspace_trash"] {
            tx.execute(&format!("DELETE FROM {table} WHERE workspace = ?1"), [workspace])?;
        }
        tx.execute("DELETE FROM workspaces WHERE id = ?1", [workspace])?;
        tx.commit()
    }

    pub fn set_key_hash(&self, workspace: &str, key_hash: &str, now_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO workspace_keys (workspace, at_ms, key_hash) VALUES (?1, ?2, ?3)", params![workspace, now_ms, key_hash])?;
        Ok(())
    }

    /// Appends one log row, plus a checkpoint when due. One transaction.
    pub fn append(&mut self, e: &LogEntry, state_after: &State) -> rusqlite::Result<()> {
        let result = serde_json::to_string(e.result).expect("applied result serializes");
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO log (workspace, seq, time_ms, actor, kind, source, grammar_version, result)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![e.workspace, e.seq as i64, e.time_ms, e.actor.as_str(), e.kind, e.source, GRAMMAR_VERSION, result],
        )?;
        if e.seq.is_multiple_of(CHECKPOINT_EVERY) {
            let state = serde_json::to_string(state_after).expect("state serializes");
            tx.execute("INSERT INTO checkpoints (workspace, seq, state) VALUES (?1, ?2, ?3)", params![e.workspace, e.seq as i64, state])?;
        }
        tx.commit()
    }

    /// Log rows of `workspace` with `seq >= from`, in order.
    pub fn entries_from(&self, workspace: &str, from: u64) -> Result<Vec<StoredEntry>, String> {
        let mut q = self.conn.prepare("SELECT seq, actor, result FROM log WHERE workspace = ?1 AND seq >= ?2 ORDER BY seq").map_err(|e| e.to_string())?;
        let rows = q
            .query_map(params![workspace, from as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?;
        let mut out = vec![];
        for row in rows {
            let (seq, actor, json) = row.map_err(|e| e.to_string())?;
            let result: AppliedResult = from_json(&json).map_err(|e| format!("log {workspace}#{seq}: {e}"))?;
            out.push(StoredEntry { seq: seq as u64, actor: Actor::parse(&actor), result });
        }
        Ok(out)
    }

    /// Restores every workspace from its nearest checkpoint plus later applied results.
    pub fn load_all(&self) -> Result<Vec<LoadedWorkspace>, String> {
        let mut out = vec![];
        let mut ws = self.conn.prepare("SELECT id, name, created_ms FROM workspaces ORDER BY created_ms, id").map_err(|e| e.to_string())?;
        let rows = ws
            .query_map([], |r| Ok(WorkspaceRow { id: r.get(0)?, name: r.get(1)?, key_hash: None, created_ms: r.get(2)?, trashed_ms: None }))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for mut row in rows {
            row.key_hash = self
                .conn
                .query_row("SELECT key_hash FROM workspace_keys WHERE workspace = ?1 ORDER BY id DESC LIMIT 1", [&row.id], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            row.trashed_ms = self
                .conn
                .query_row("SELECT at_ms, trashed FROM workspace_trash WHERE workspace = ?1 ORDER BY id DESC LIMIT 1", [&row.id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .optional()
                .map_err(|e| e.to_string())?
                .and_then(|(at, t)| (t != 0).then_some(at));
            let cp: Option<(i64, String)> = self
                .conn
                .query_row("SELECT seq, state FROM checkpoints WHERE workspace = ?1 ORDER BY seq DESC LIMIT 1", [&row.id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(|e| e.to_string())?;
            let (mut state, from) = match cp {
                Some((seq, json)) => (from_json::<State>(&json).map_err(|e| format!("checkpoint {}#{seq}: {e}", row.id))?, seq),
                None => (State::default(), 0),
            };
            let mut revision = from as u64;
            let mut updated_ms = row.created_ms;
            let mut last_notes = vec![];
            let mut q = self
                .conn
                .prepare("SELECT seq, time_ms, result FROM log WHERE workspace = ?1 AND seq > ?2 ORDER BY seq")
                .map_err(|e| e.to_string())?;
            let entries = q
                .query_map(params![row.id, from], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))
                .map_err(|e| e.to_string())?;
            for entry in entries {
                let (seq, time_ms, json) = entry.map_err(|e| e.to_string())?;
                let result: AppliedResult = from_json(&json).map_err(|e| format!("log {}#{seq}: {e}", row.id))?;
                state.apply_changes(&result.changes);
                revision = seq as u64;
                updated_ms = time_ms;
                last_notes = result.notes;
            }
            if from > 0 && revision == from as u64 {
                // Only the checkpoint: take time and notes from its log row.
                if let Some((t, json)) = self
                    .conn
                    .query_row("SELECT time_ms, result FROM log WHERE workspace = ?1 AND seq = ?2", params![row.id, from], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))
                    .optional()
                    .map_err(|e| e.to_string())?
                {
                    updated_ms = t;
                    last_notes = from_json::<AppliedResult>(&json).map(|r| r.notes).unwrap_or_default();
                }
            }
            out.push(LoadedWorkspace { row, state, revision, updated_ms, last_notes });
        }
        Ok(out)
    }
}
