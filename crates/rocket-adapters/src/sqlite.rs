//! SQLite-backed [`ports::Store`] (Go: `adapters/sqlite`).
//!
//! The schema, pragmas, column formats and JSON `data` blobs are identical to
//! the Go adapter, so a `state.db` written by either implementation is read
//! by the other:
//!
//! * `projects.added_at` and `leases.created_at` are Go `RFC3339Nano` strings
//!   (UTC, trailing fractional zeros trimmed);
//! * `jobs.started_at` / `jobs.finished_at` use a fixed-width nanosecond
//!   format so lexical order equals time order;
//! * `runs.data` and `jobs.data` hold the domain types encoded by
//!   `serde_json`, which matches Go's `encoding/json` byte for byte.
//!
//! One difference: Go distinguishes a nil slice (`null`) from an empty one
//! (`[]`) in `jobs.steps`; Rust always writes `[]`, which Go decodes the same.

use rocket_domain::ports::{self, Error, Result};
use rocket_domain::{Job, Lease, ProjectRef, Run};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use time::OffsetDateTime;
use time::format_description::BorrowedFormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::{datetime, format_description};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
  name     TEXT PRIMARY KEY,
  path     TEXT NOT NULL,
  added_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
  project TEXT NOT NULL,
  service TEXT NOT NULL,
  data    TEXT NOT NULL,
  PRIMARY KEY (project, service)
);
CREATE TABLE IF NOT EXISTS jobs (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL,
  owner       TEXT NOT NULL,
  steps       TEXT NOT NULL,
  status      TEXT NOT NULL,
  exit_code   INTEGER,
  started_at  TEXT NOT NULL,
  finished_at TEXT,
  log_path    TEXT NOT NULL,
  data        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS jobs_project_started ON jobs(project, started_at);
CREATE TABLE IF NOT EXISTS leases (
  port       INTEGER PRIMARY KEY,
  project    TEXT NOT NULL,
  service    TEXT NOT NULL,
  port_name  TEXT NOT NULL,
  created_at TEXT NOT NULL
);
";

/// Fixed-width so lexical order equals time order (Go: `sortableTime`).
const SORTABLE_TIME: &[BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z");

/// A SQLite [`ports::Store`]. One connection behind a mutex, like Go's
/// `SetMaxOpenConns(1)`.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (and migrates) the database at `path`: `busy_timeout=5000`,
    /// WAL journal and foreign keys on, as in Go.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(Error::other)?;
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(Error::other)?;
        // `journal_mode` returns the resulting mode as a row.
        conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))
            .map_err(Error::other)?;
        conn.pragma_update(None, "foreign_keys", 1)
            .map_err(Error::other)?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::msg(format!("migrate {}: {e}", path.display())))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn db(e: rusqlite::Error) -> Error {
    Error::other(e)
}

fn json(e: serde_json::Error) -> Error {
    Error::other(e)
}

/// Go `time.RFC3339Nano` in UTC.
fn format_rfc3339_nano(t: OffsetDateTime) -> Result<String> {
    t.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(Error::other)
}

/// Go ignores parse errors and keeps the zero time; so do we.
fn parse_rfc3339_nano(s: &str) -> OffsetDateTime {
    OffsetDateTime::parse(s, &Rfc3339).unwrap_or(datetime!(0001-01-01 00:00:00 UTC))
}

fn format_sortable(t: OffsetDateTime) -> Result<String> {
    t.to_offset(time::UtcOffset::UTC)
        .format(&SORTABLE_TIME)
        .map_err(Error::other)
}

fn lease_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Lease> {
    let created: String = r.get(4)?;
    Ok(Lease {
        port: r.get(0)?,
        project: r.get(1)?,
        service: r.get(2)?,
        port_name: r.get(3)?,
        created_at: parse_rfc3339_nano(&created),
    })
}

fn query_leases(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Lease>> {
    let mut stmt = conn.prepare(sql).map_err(db)?;
    stmt.query_map(params, lease_from_row)
        .map_err(db)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db)
}

fn query_data<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql).map_err(db)?;
    let rows = stmt
        .query_map(params, |r| r.get::<_, String>(0))
        .map_err(db)?;
    let mut out = Vec::new();
    for data in rows {
        out.push(serde_json::from_str(&data.map_err(db)?).map_err(json)?);
    }
    Ok(out)
}

fn get_data<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<T>> {
    let data: Option<String> = conn
        .query_row(sql, params, |r| r.get(0))
        .optional()
        .map_err(db)?;
    data.map(|d| serde_json::from_str(&d).map_err(json))
        .transpose()
}

impl ports::Store for Store {
    fn upsert_project(&self, p: &ProjectRef) -> Result<()> {
        let added = format_rfc3339_nano(p.added_at)?;
        self.conn()
            .execute(
                "INSERT INTO projects(name, path, added_at) VALUES(?,?,?)
                 ON CONFLICT(name) DO UPDATE SET path=excluded.path",
                params![p.name, p.path, added],
            )
            .map_err(db)?;
        Ok(())
    }

    fn get_project(&self, name: &str) -> Result<Option<ProjectRef>> {
        self.conn()
            .query_row(
                "SELECT name, path, added_at FROM projects WHERE name=?",
                [name],
                project_from_row,
            )
            .optional()
            .map_err(db)
    }

    fn list_projects(&self) -> Result<Vec<ProjectRef>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT name, path, added_at FROM projects ORDER BY name")
            .map_err(db)?;
        stmt.query_map([], project_from_row)
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    fn delete_project(&self, name: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM projects WHERE name=?", [name])
            .map_err(db)?;
        Ok(())
    }

    fn save_run(&self, run: &Run) -> Result<()> {
        let data = serde_json::to_string(run).map_err(json)?;
        self.conn()
            .execute(
                "INSERT INTO runs(project, service, data) VALUES(?,?,?)
                 ON CONFLICT(project, service) DO UPDATE SET data=excluded.data",
                params![run.project, run.service, data],
            )
            .map_err(db)?;
        Ok(())
    }

    fn get_run(&self, project: &str, service: &str) -> Result<Option<Run>> {
        get_data(
            &self.conn(),
            "SELECT data FROM runs WHERE project=? AND service=?",
            [project, service],
        )
    }

    fn list_runs(&self) -> Result<Vec<Run>> {
        query_data(
            &self.conn(),
            "SELECT data FROM runs ORDER BY project, service",
            [],
        )
    }

    fn delete_run(&self, project: &str, service: &str) -> Result<()> {
        self.conn()
            .execute(
                "DELETE FROM runs WHERE project=? AND service=?",
                [project, service],
            )
            .map_err(db)?;
        Ok(())
    }

    fn acquire_lease(&self, l: &Lease) -> Result<()> {
        let created = format_rfc3339_nano(l.created_at)?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(db)?;
        let holder: Option<(String, String)> = tx
            .query_row(
                "SELECT project, service FROM leases WHERE port=?",
                [l.port],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        if let Some((project, service)) = holder
            && (project != l.project || service != l.service)
        {
            // Go wraps the sentinel with "port N held by project/service";
            // the Rust sentinel carries no payload.
            return Err(Error::LeaseTaken);
        }
        tx.execute(
            "INSERT INTO leases(port, project, service, port_name, created_at) VALUES(?,?,?,?,?)
             ON CONFLICT(port) DO UPDATE SET port_name=excluded.port_name, created_at=excluded.created_at",
            params![l.port, l.project, l.service, l.port_name, created],
        )
        .map_err(db)?;
        tx.commit().map_err(db)
    }

    fn release_leases(&self, project: &str, service: &str) -> Result<Vec<Lease>> {
        let conn = self.conn();
        let leases = query_leases(
            &conn,
            "SELECT port, project, service, port_name, created_at FROM leases WHERE project=? AND service=? ORDER BY port",
            [project, service],
        )?;
        conn.execute(
            "DELETE FROM leases WHERE project=? AND service=?",
            [project, service],
        )
        .map_err(db)?;
        Ok(leases)
    }

    fn list_leases(&self) -> Result<Vec<Lease>> {
        query_leases(
            &self.conn(),
            "SELECT port, project, service, port_name, created_at FROM leases ORDER BY port",
            [],
        )
    }

    fn save_job(&self, j: &Job) -> Result<()> {
        let data = serde_json::to_string(j).map_err(json)?;
        let steps = serde_json::to_string(&j.steps).map_err(json)?;
        let finished = j.finished_at.map(format_sortable).transpose()?;
        let started = format_sortable(j.started_at)?;
        self.conn()
            .execute(
                "INSERT INTO jobs(id, project, name, kind, owner, steps, status, exit_code, started_at, finished_at, log_path, data)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET status=excluded.status, exit_code=excluded.exit_code,
                   finished_at=excluded.finished_at, log_path=excluded.log_path, data=excluded.data",
                params![
                    j.id,
                    j.project,
                    j.name,
                    j.kind.as_str(),
                    j.owner,
                    steps,
                    j.status.as_str(),
                    j.exit_code,
                    started,
                    finished,
                    j.log_path,
                    data
                ],
            )
            .map_err(db)?;
        Ok(())
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>> {
        get_data(&self.conn(), "SELECT data FROM jobs WHERE id=?", [id])
    }

    fn list_jobs(&self, project: &str, limit: i64) -> Result<Vec<Job>> {
        let mut sql = String::from("SELECT data FROM jobs");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if !project.is_empty() {
            sql.push_str(" WHERE project=?");
            args.push(project.to_string().into());
        }
        sql.push_str(" ORDER BY started_at DESC, id DESC");
        if limit > 0 {
            sql.push_str(" LIMIT ?");
            args.push(limit.into());
        }
        query_data(&self.conn(), &sql, rusqlite::params_from_iter(args))
    }

    fn delete_job(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM jobs WHERE id=?", [id])
            .map_err(db)?;
        Ok(())
    }
}

fn project_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRef> {
    let added: String = r.get(2)?;
    Ok(ProjectRef {
        name: r.get(0)?,
        path: r.get(1)?,
        added_at: parse_rfc3339_nano(&added),
    })
}
