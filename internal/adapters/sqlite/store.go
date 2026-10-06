// Package sqlite implements ports.Store on SQLite (modernc.org/sqlite, no CGO).
package sqlite

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	_ "modernc.org/sqlite" // registers the "sqlite" driver

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

const schema = `
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
`

// Store is a SQLite-backed ports.Store.
type Store struct {
	db *sql.DB
}

var _ ports.Store = (*Store)(nil)

// Open opens (and migrates) the database at path.
func Open(path string) (*Store, error) {
	dsn := fmt.Sprintf("file:%s?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=foreign_keys(1)", path)
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, fmt.Errorf("migrate %s: %w", path, err)
	}
	return &Store{db: db}, nil
}

// Close closes the database.
func (s *Store) Close() error { return s.db.Close() }

func (s *Store) UpsertProject(r domain.ProjectRef) error {
	_, err := s.db.Exec(`INSERT INTO projects(name, path, added_at) VALUES(?,?,?)
		ON CONFLICT(name) DO UPDATE SET path=excluded.path`, r.Name, r.Path, r.AddedAt.UTC().Format(time.RFC3339Nano))
	return err
}

func (s *Store) GetProject(name string) (domain.ProjectRef, bool, error) {
	var r domain.ProjectRef
	var added string
	err := s.db.QueryRow(`SELECT name, path, added_at FROM projects WHERE name=?`, name).Scan(&r.Name, &r.Path, &added)
	if errors.Is(err, sql.ErrNoRows) {
		return r, false, nil
	}
	if err != nil {
		return r, false, err
	}
	r.AddedAt, _ = time.Parse(time.RFC3339Nano, added)
	return r, true, nil
}

func (s *Store) ListProjects() ([]domain.ProjectRef, error) {
	rows, err := s.db.Query(`SELECT name, path, added_at FROM projects ORDER BY name`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []domain.ProjectRef{}
	for rows.Next() {
		var r domain.ProjectRef
		var added string
		if err := rows.Scan(&r.Name, &r.Path, &added); err != nil {
			return nil, err
		}
		r.AddedAt, _ = time.Parse(time.RFC3339Nano, added)
		out = append(out, r)
	}
	return out, rows.Err()
}

func (s *Store) DeleteProject(name string) error {
	_, err := s.db.Exec(`DELETE FROM projects WHERE name=?`, name)
	return err
}

func (s *Store) SaveRun(r domain.Run) error {
	data, err := json.Marshal(r)
	if err != nil {
		return err
	}
	_, err = s.db.Exec(`INSERT INTO runs(project, service, data) VALUES(?,?,?)
		ON CONFLICT(project, service) DO UPDATE SET data=excluded.data`, r.Project, r.Service, string(data))
	return err
}

func (s *Store) GetRun(project, service string) (domain.Run, bool, error) {
	var data string
	err := s.db.QueryRow(`SELECT data FROM runs WHERE project=? AND service=?`, project, service).Scan(&data)
	if errors.Is(err, sql.ErrNoRows) {
		return domain.Run{}, false, nil
	}
	if err != nil {
		return domain.Run{}, false, err
	}
	var r domain.Run
	if err := json.Unmarshal([]byte(data), &r); err != nil {
		return domain.Run{}, false, err
	}
	return r, true, nil
}

func (s *Store) ListRuns() ([]domain.Run, error) {
	rows, err := s.db.Query(`SELECT data FROM runs ORDER BY project, service`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []domain.Run{}
	for rows.Next() {
		var data string
		if err := rows.Scan(&data); err != nil {
			return nil, err
		}
		var r domain.Run
		if err := json.Unmarshal([]byte(data), &r); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

func (s *Store) DeleteRun(project, service string) error {
	_, err := s.db.Exec(`DELETE FROM runs WHERE project=? AND service=?`, project, service)
	return err
}

func (s *Store) AcquireLease(l domain.Lease) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var project, service string
	err = tx.QueryRow(`SELECT project, service FROM leases WHERE port=?`, l.Port).Scan(&project, &service)
	switch {
	case errors.Is(err, sql.ErrNoRows):
	case err != nil:
		return err
	case project != l.Project || service != l.Service:
		return fmt.Errorf("%w: port %d held by %s/%s", ports.ErrLeaseTaken, l.Port, project, service)
	}
	if _, err := tx.Exec(`INSERT INTO leases(port, project, service, port_name, created_at) VALUES(?,?,?,?,?)
		ON CONFLICT(port) DO UPDATE SET port_name=excluded.port_name, created_at=excluded.created_at`,
		l.Port, l.Project, l.Service, l.PortName, l.CreatedAt.UTC().Format(time.RFC3339Nano)); err != nil {
		return err
	}
	return tx.Commit()
}

func (s *Store) ReleaseLeases(project, service string) ([]domain.Lease, error) {
	leases, err := s.queryLeases(`SELECT port, project, service, port_name, created_at FROM leases WHERE project=? AND service=? ORDER BY port`, project, service)
	if err != nil {
		return nil, err
	}
	if _, err := s.db.Exec(`DELETE FROM leases WHERE project=? AND service=?`, project, service); err != nil {
		return nil, err
	}
	return leases, nil
}

func (s *Store) ListLeases() ([]domain.Lease, error) {
	return s.queryLeases(`SELECT port, project, service, port_name, created_at FROM leases ORDER BY port`)
}

func (s *Store) queryLeases(q string, args ...any) ([]domain.Lease, error) {
	rows, err := s.db.Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []domain.Lease{}
	for rows.Next() {
		var l domain.Lease
		var created string
		if err := rows.Scan(&l.Port, &l.Project, &l.Service, &l.PortName, &created); err != nil {
			return nil, err
		}
		l.CreatedAt, _ = time.Parse(time.RFC3339Nano, created)
		out = append(out, l)
	}
	return out, rows.Err()
}

// sortableTime is fixed-width so lexical order equals time order.
const sortableTime = "2006-01-02T15:04:05.000000000Z"

// SaveJob upserts a job. Queryable fields are columns; data holds the full
// record.
func (s *Store) SaveJob(j domain.Job) error {
	data, err := json.Marshal(j)
	if err != nil {
		return err
	}
	steps, err := json.Marshal(j.Steps)
	if err != nil {
		return err
	}
	var finished any
	if j.FinishedAt != nil {
		finished = j.FinishedAt.UTC().Format(sortableTime)
	}
	var code any
	if j.ExitCode != nil {
		code = *j.ExitCode
	}
	_, err = s.db.Exec(`INSERT INTO jobs(id, project, name, kind, owner, steps, status, exit_code, started_at, finished_at, log_path, data)
		VALUES(?,?,?,?,?,?,?,?,?,?,?,?)
		ON CONFLICT(id) DO UPDATE SET status=excluded.status, exit_code=excluded.exit_code,
		  finished_at=excluded.finished_at, log_path=excluded.log_path, data=excluded.data`,
		j.ID, j.Project, j.Name, string(j.Kind), j.Owner, string(steps), string(j.Status), code,
		j.StartedAt.UTC().Format(sortableTime), finished, j.LogPath, string(data))
	return err
}

func (s *Store) GetJob(id string) (domain.Job, bool, error) {
	var data string
	err := s.db.QueryRow(`SELECT data FROM jobs WHERE id=?`, id).Scan(&data)
	if errors.Is(err, sql.ErrNoRows) {
		return domain.Job{}, false, nil
	}
	if err != nil {
		return domain.Job{}, false, err
	}
	var j domain.Job
	if err := json.Unmarshal([]byte(data), &j); err != nil {
		return domain.Job{}, false, err
	}
	return j, true, nil
}

func (s *Store) ListJobs(project string, limit int) ([]domain.Job, error) {
	q := `SELECT data FROM jobs`
	var args []any
	if project != "" {
		q += ` WHERE project=?`
		args = append(args, project)
	}
	q += ` ORDER BY started_at DESC, id DESC`
	if limit > 0 {
		q += ` LIMIT ?`
		args = append(args, limit)
	}
	rows, err := s.db.Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []domain.Job{}
	for rows.Next() {
		var data string
		if err := rows.Scan(&data); err != nil {
			return nil, err
		}
		var j domain.Job
		if err := json.Unmarshal([]byte(data), &j); err != nil {
			return nil, err
		}
		out = append(out, j)
	}
	return out, rows.Err()
}

func (s *Store) DeleteJob(id string) error {
	_, err := s.db.Exec(`DELETE FROM jobs WHERE id=?`, id)
	return err
}
