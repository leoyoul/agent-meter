use crate::models::{
    MetricFilters, ModelStat, Overview, SourceInfo, TaskRow, TimeseriesPoint, TokenBreakdown,
};
use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub type AppResult<T> = Result<T, String>;

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path).map_err(to_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))
        .map_err(to_error)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;",
    )
    .map_err(to_error)?;
    Ok(conn)
}

pub fn migrate(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(to_error)?;
    }
    let conn = open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            root_path TEXT NOT NULL UNIQUE,
            source_kind TEXT NOT NULL DEFAULT 'codex_jsonl',
            enabled INTEGER NOT NULL DEFAULT 1,
            last_scan_at TEXT,
            error TEXT
         );
         CREATE TABLE IF NOT EXISTS scan_files (
            id INTEGER PRIMARY KEY,
            source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            path TEXT NOT NULL UNIQUE,
            size INTEGER NOT NULL DEFAULT 0,
            mtime_ms INTEGER NOT NULL DEFAULT 0,
            offset INTEGER NOT NULL DEFAULT 0,
            current_session_id TEXT,
            current_turn_id TEXT,
            last_input INTEGER NOT NULL DEFAULT 0,
            last_cached_input INTEGER NOT NULL DEFAULT 0,
            last_output INTEGER NOT NULL DEFAULT 0,
            last_reasoning INTEGER NOT NULL DEFAULT 0,
            last_total INTEGER NOT NULL DEFAULT 0,
            last_scan_at TEXT,
            error TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_scan_files_source ON scan_files(source_id);
         CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            source_id INTEGER NOT NULL REFERENCES sources(id),
            file_path TEXT NOT NULL,
            parent_thread_id TEXT,
            thread_source TEXT,
            agent_path TEXT,
            agent_kind TEXT NOT NULL DEFAULT 'root',
            cwd TEXT NOT NULL DEFAULT '',
            project TEXT NOT NULL DEFAULT '',
            started_at TEXT,
            updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source_id);
         CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project);
         CREATE TABLE IF NOT EXISTS turns (
            turn_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            source_id INTEGER NOT NULL REFERENCES sources(id),
            started_at TEXT NOT NULL,
            started_local_date TEXT NOT NULL,
            completed_at TEXT,
            duration_ms INTEGER,
            ttft_ms INTEGER,
            model TEXT NOT NULL DEFAULT 'unknown',
            reasoning_effort TEXT,
            cwd TEXT NOT NULL DEFAULT '',
            project TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'running',
            input_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_turns_started_date ON turns(started_local_date);
         CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id);
         CREATE INDEX IF NOT EXISTS idx_turns_model ON turns(model);
         CREATE TABLE IF NOT EXISTS model_calls (
            response_id TEXT PRIMARY KEY,
            turn_id TEXT NOT NULL REFERENCES turns(turn_id) ON DELETE CASCADE,
            session_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            call_kind TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            cached_input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_calls_turn ON model_calls(turn_id);
         CREATE INDEX IF NOT EXISTS idx_calls_kind ON model_calls(turn_id, call_kind);",
    )
    .map_err(to_error)?;
    if !has_column(&conn, "sources", "source_kind")? {
        conn.execute(
            "ALTER TABLE sources ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'codex_jsonl'",
            [],
        )
        .map_err(to_error)?;
    }
    if !has_column(&conn, "scan_files", "parse_error_count")? {
        conn.execute(
            "ALTER TABLE scan_files ADD COLUMN parse_error_count INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(to_error)?;
    }
    sync_known_sources(&conn)?;
    conn.execute(
        "UPDATE sources SET enabled=0 WHERE name IN ('Yodex','Lodex')",
        [],
    )
    .map_err(to_error)?;
    crate::analytics::migrate(&conn)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(to_error)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(to_error)?;
    for name in names {
        if name.map_err(to_error)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn reset_index(path: &Path) -> AppResult<()> {
    let conn = open(path)?;
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DELETE FROM model_calls;
         DELETE FROM turns;
         DELETE FROM sessions;
         DELETE FROM scan_files;
         DELETE FROM usage_observations;
         DELETE FROM model_call_observations;
         DELETE FROM performance_observations;
         DELETE FROM source_sync_cursors;
         UPDATE sources SET last_scan_at=NULL, error=NULL;
         COMMIT;",
    )
    .map_err(to_error)
}

fn known_source_paths() -> Vec<(String, PathBuf, &'static str)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        ("Codex".into(), home.join(".codex"), "codex_jsonl"),
        (
            "ZCode".into(),
            home.join(".zcode/cli/db/db.sqlite"),
            "zcode_sqlite",
        ),
        (
            "OpenCode".into(),
            home.join(".local/share/opencode/opencode.db"),
            "opencode_sqlite",
        ),
        ("DSH".into(), home.join(".dsh/sessions"), "dsh_zstd"),
        (
            "Claude".into(),
            home.join("Library/Application Support/Claude"),
            "claude_desktop",
        ),
        (
            "EvoX".into(),
            home.join(".evox/agent/observability"),
            "evox_observability",
        ),
    ]
}

pub fn sync_known_sources(conn: &Connection) -> AppResult<()> {
    for (name, path, source_kind) in known_source_paths() {
        conn.execute(
            "INSERT INTO sources(name, root_path, source_kind, enabled)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(root_path) DO UPDATE SET name=excluded.name,source_kind=excluded.source_kind",
            params![name, path.to_string_lossy(), source_kind],
        )
        .map_err(to_error)?;
    }
    Ok(())
}

pub fn discover_sources(path: &Path) -> AppResult<Vec<SourceInfo>> {
    let conn = open(path)?;
    sync_known_sources(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, root_path, source_kind, enabled, last_scan_at, error
             FROM sources WHERE name NOT IN ('Yodex','Lodex') ORDER BY id",
        )
        .map_err(to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(to_error)?;
    let mut result = Vec::new();
    for row in rows {
        let (id, name, root_path, source_kind, enabled, last_scan_at, error) =
            row.map_err(to_error)?;
        let source_path = Path::new(&root_path);
        let available = if matches!(
            source_kind.as_str(),
            "codex_jsonl" | "dsh_zstd" | "claude_desktop" | "evox_observability"
        ) {
            source_path.is_dir()
        } else {
            source_path.is_file()
        };
        let (file_count, total_bytes) = if source_kind == "codex_jsonl" {
            source_inventory(source_path)
        } else if source_kind == "dsh_zstd" {
            extension_inventory(source_path, "zstd")
        } else if source_kind == "evox_observability" {
            extension_inventory(source_path, "jsonl")
        } else if let Ok(metadata) = source_path.metadata() {
            (1, metadata.len())
        } else {
            (0, 0)
        };
        result.push(SourceInfo {
            id,
            name,
            source_kind: source_kind.clone(),
            root_path,
            enabled,
            available,
            file_count,
            total_bytes,
            last_scan_at,
            error,
            data_capability: if source_kind == "claude_desktop" {
                "noUsageLog".into()
            } else {
                "metrics".into()
            },
            limitation: if source_kind == "claude_desktop" {
                Some("未发现可统计的本地 Token 记录".into())
            } else {
                None
            },
        });
    }
    Ok(result)
}

fn extension_inventory(root: &Path, extension: &str) -> (u64, u64) {
    if !root.is_dir() {
        return (0, 0);
    }
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
        })
        .fold((0_u64, 0_u64), |(count, bytes), entry| {
            (
                count + 1,
                bytes.saturating_add(entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)),
            )
        })
}

pub fn source_inventory(root: &Path) -> (u64, u64) {
    if !root.is_dir() {
        return (0, 0);
    }
    let mut count = 0_u64;
    let mut bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() && is_rollout(entry.path()) {
            count += 1;
            bytes = bytes.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    (count, bytes)
}

pub fn is_rollout(path: &Path) -> bool {
    path.extension().and_then(|v| v.to_str()) == Some("jsonl")
        && path
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|name| name.starts_with("rollout-"))
}

pub fn enabled_sources(path: &Path) -> AppResult<Vec<(i64, PathBuf)>> {
    let conn = open(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, root_path FROM sources
             WHERE enabled=1 AND source_kind='codex_jsonl' ORDER BY id",
        )
        .map_err(to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, PathBuf::from(row.get::<_, String>(1)?)))
        })
        .map_err(to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(to_error)
}

pub fn update_source(path: &Path, source_id: i64, enabled: bool) -> AppResult<SourceInfo> {
    let conn = open(path)?;
    let changed = conn
        .execute(
            "UPDATE sources SET enabled=?1 WHERE id=?2",
            params![enabled, source_id],
        )
        .map_err(to_error)?;
    if changed == 0 {
        return Err(format!("source {source_id} not found"));
    }
    discover_sources(path)?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| format!("source {source_id} not found"))
}

pub fn source_scope(
    path: &Path,
    source_kind: &str,
) -> AppResult<Option<(i64, String, bool, String)>> {
    let conn = open(path)?;
    conn.query_row(
        "SELECT id,name,enabled,root_path FROM sources WHERE source_kind=?1 LIMIT 1",
        [source_kind],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .optional()
    .map_err(to_error)
}

#[derive(Debug, Clone)]
struct MetricRow {
    bucket: String,
    model: String,
    effort: Option<String>,
    tokens: TokenBreakdown,
    ttft: Option<f64>,
    duration: Option<f64>,
    tps: Option<f64>,
    status: String,
    agent_kind: String,
    updated_at: String,
}

fn filter_sql(filters: &MetricFilters) -> (String, Vec<Box<dyn ToSql>>) {
    let mut where_parts = vec!["1=1".to_string()];
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(value) = filters.start_date.as_ref() {
        where_parts.push("t.started_local_date >= ?".into());
        values.push(Box::new(value.clone()));
    }
    if let Some(value) = filters.end_date.as_ref() {
        where_parts.push("t.started_local_date <= ?".into());
        values.push(Box::new(value.clone()));
    }
    if let Some(value) = filters.source_id {
        where_parts.push("t.source_id = ?".into());
        values.push(Box::new(value));
    }
    if let Some(value) = filters.model.as_ref().filter(|v| !v.is_empty()) {
        where_parts.push("t.model = ?".into());
        values.push(Box::new(value.clone()));
    }
    if let Some(value) = filters.project.as_ref().filter(|v| !v.is_empty()) {
        where_parts.push("t.project = ?".into());
        values.push(Box::new(value.clone()));
    }
    if let Some(value) = filters
        .agent_kind
        .as_ref()
        .filter(|v| v.as_str() == "root" || v.as_str() == "subagent")
    {
        where_parts.push("s.agent_kind = ?".into());
        values.push(Box::new(value.clone()));
    }
    (where_parts.join(" AND "), values)
}

fn metric_rows(conn: &Connection, filters: &MetricFilters) -> AppResult<Vec<MetricRow>> {
    let (where_sql, values) = filter_sql(filters);
    let sql = format!(
        "SELECT t.started_local_date, t.model, t.reasoning_effort,
                t.input_tokens, t.cached_input_tokens, t.output_tokens,
                t.reasoning_tokens, t.total_tokens, t.ttft_ms, t.duration_ms,
                CASE WHEN t.status='completed' AND t.duration_ms > COALESCE(t.ttft_ms, 0)
                     THEN CAST(t.output_tokens AS REAL) / ((t.duration_ms - COALESCE(t.ttft_ms, 0)) / 1000.0)
                END,
                t.status, s.agent_kind, t.updated_at
         FROM turns t JOIN sessions s ON s.session_id=t.session_id
         WHERE {where_sql}"
    );
    let params: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(to_error)?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            Ok(MetricRow {
                bucket: row.get(0)?,
                model: row.get(1)?,
                effort: row.get(2)?,
                tokens: TokenBreakdown {
                    input: row.get(3)?,
                    cached_input: row.get(4)?,
                    output: row.get(5)?,
                    reasoning: row.get(6)?,
                    total: row.get(7)?,
                },
                ttft: row.get::<_, Option<i64>>(8)?.map(|v| v as f64),
                duration: row.get::<_, Option<i64>>(9)?.map(|v| v as f64),
                tps: row.get(10)?,
                status: row.get(11)?,
                agent_kind: row.get(12)?,
                updated_at: row.get(13)?,
            })
        })
        .map_err(to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(to_error)
}

pub fn query_overview(path: &Path, filters: MetricFilters) -> AppResult<Overview> {
    let conn = open(path)?;
    let rows = metric_rows(&conn, &filters)?;
    let mut tokens = TokenBreakdown::default();
    let mut ttft = Vec::new();
    let mut duration = Vec::new();
    let mut tps = Vec::new();
    let mut recent: Vec<&MetricRow> = rows.iter().filter(|r| r.status == "completed").collect();
    recent.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    recent.truncate(5);
    for row in &rows {
        tokens += row.tokens;
        push_some(&mut ttft, row.ttft);
        push_some(&mut duration, row.duration);
        push_some(&mut tps, row.tps);
    }
    let mut recent_ttft = recent.iter().filter_map(|r| r.ttft).collect::<Vec<_>>();
    let mut recent_tps = recent.iter().filter_map(|r| r.tps).collect::<Vec<_>>();
    Ok(Overview {
        tokens,
        turn_count: rows.len() as i64,
        running_count: rows.iter().filter(|r| r.status == "running").count() as i64,
        subagent_count: rows.iter().filter(|r| r.agent_kind == "subagent").count() as i64,
        median_ttft_ms: percentile(&mut ttft, 0.5),
        p95_ttft_ms: percentile(&mut ttft, 0.95),
        median_duration_ms: percentile(&mut duration, 0.5),
        p95_duration_ms: percentile(&mut duration, 0.95),
        median_effective_tps: percentile(&mut tps, 0.5),
        recent_median_ttft_ms: percentile(&mut recent_ttft, 0.5),
        recent_median_effective_tps: percentile(&mut recent_tps, 0.5),
        last_updated_at: rows.iter().map(|r| r.updated_at.clone()).max(),
    })
}

pub fn query_timeseries(path: &Path, filters: MetricFilters) -> AppResult<Vec<TimeseriesPoint>> {
    let conn = open(path)?;
    let rows = metric_rows(&conn, &filters)?;
    let mut groups: BTreeMap<String, Vec<MetricRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(row.bucket.clone()).or_default().push(row);
    }
    Ok(groups
        .into_iter()
        .map(|(bucket, rows)| {
            let mut tokens = TokenBreakdown::default();
            let mut ttft = Vec::new();
            let mut tps = Vec::new();
            for row in &rows {
                tokens += row.tokens;
                push_some(&mut ttft, row.ttft);
                push_some(&mut tps, row.tps);
            }
            TimeseriesPoint {
                bucket,
                input_tokens: tokens.input,
                cached_input_tokens: tokens.cached_input,
                output_tokens: tokens.output,
                reasoning_tokens: tokens.reasoning,
                total_tokens: tokens.total,
                median_ttft_ms: percentile(&mut ttft, 0.5),
                median_effective_tps: percentile(&mut tps, 0.5),
                turn_count: rows.len() as i64,
            }
        })
        .collect())
}

pub fn query_model_stats(path: &Path, filters: MetricFilters) -> AppResult<Vec<ModelStat>> {
    let conn = open(path)?;
    let rows = metric_rows(&conn, &filters)?;
    let mut groups: BTreeMap<(String, Option<String>), Vec<MetricRow>> = BTreeMap::new();
    for row in rows {
        groups
            .entry((row.model.clone(), row.effort.clone()))
            .or_default()
            .push(row);
    }
    let mut result = groups
        .into_iter()
        .map(|((model, effort), rows)| {
            let mut tokens = TokenBreakdown::default();
            let mut ttft = Vec::new();
            let mut duration = Vec::new();
            let mut tps = Vec::new();
            for row in &rows {
                tokens += row.tokens;
                push_some(&mut ttft, row.ttft);
                push_some(&mut duration, row.duration);
                push_some(&mut tps, row.tps);
            }
            ModelStat {
                model,
                reasoning_effort: effort,
                turn_count: rows.len() as i64,
                tokens,
                median_ttft_ms: percentile(&mut ttft, 0.5),
                p95_ttft_ms: percentile(&mut ttft, 0.95),
                median_duration_ms: percentile(&mut duration, 0.5),
                p95_duration_ms: percentile(&mut duration, 0.95),
                median_effective_tps: percentile(&mut tps, 0.5),
            }
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|row| std::cmp::Reverse(row.tokens.total));
    Ok(result)
}

pub fn query_tasks(
    path: &Path,
    filters: MetricFilters,
    limit: Option<i64>,
) -> AppResult<Vec<TaskRow>> {
    let conn = open(path)?;
    let (where_sql, mut values) = filter_sql(&filters);
    let limit = limit.unwrap_or(100).clamp(1, 500);
    values.push(Box::new(limit));
    let sql = format!(
        "SELECT t.turn_id, t.session_id, s.parent_thread_id, src.name,
                t.project, t.cwd, t.model, t.reasoning_effort, s.agent_kind,
                s.agent_path, t.started_at, t.completed_at, t.duration_ms, t.ttft_ms,
                CASE WHEN t.status='completed' AND t.duration_ms > COALESCE(t.ttft_ms, 0)
                     THEN CAST(t.output_tokens AS REAL) / ((t.duration_ms - COALESCE(t.ttft_ms, 0)) / 1000.0)
                END,
                t.input_tokens, t.cached_input_tokens, t.output_tokens,
                t.reasoning_tokens, t.total_tokens, t.status
         FROM turns t
         JOIN sessions s ON s.session_id=t.session_id
         JOIN sources src ON src.id=t.source_id
         WHERE {where_sql}
         ORDER BY t.started_at DESC LIMIT ?"
    );
    let params: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(to_error)?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            Ok(TaskRow {
                turn_id: row.get(0)?,
                session_id: row.get(1)?,
                parent_thread_id: row.get(2)?,
                source_name: row.get(3)?,
                project: row.get(4)?,
                cwd: row.get(5)?,
                model: row.get(6)?,
                reasoning_effort: row.get(7)?,
                agent_kind: row.get(8)?,
                agent_path: row.get(9)?,
                started_at: row.get(10)?,
                completed_at: row.get(11)?,
                duration_ms: row.get(12)?,
                ttft_ms: row.get(13)?,
                effective_tps: row.get(14)?,
                tokens: TokenBreakdown {
                    input: row.get(15)?,
                    cached_input: row.get(16)?,
                    output: row.get(17)?,
                    reasoning: row.get(18)?,
                    total: row.get(19)?,
                },
                status: row.get(20)?,
            })
        })
        .map_err(to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(to_error)
}

pub fn database_last_updated(path: &Path) -> Option<String> {
    let conn = open(path).ok()?;
    conn.query_row("SELECT MAX(updated_at) FROM turns", [], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
}

fn push_some(values: &mut Vec<f64>, value: Option<f64>) {
    if let Some(value) = value.filter(|v| v.is_finite() && *v >= 0.0) {
        values.push(value);
    }
}

pub fn percentile(values: &mut [f64], quantile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let position = ((values.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).ceil() as usize;
    Some(values[position])
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn to_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn percentile_uses_nearest_rank() {
        let mut values = vec![50.0, 10.0, 30.0, 20.0, 40.0];
        assert_eq!(percentile(&mut values, 0.5), Some(30.0));
        assert_eq!(percentile(&mut values, 0.95), Some(50.0));
    }

    #[test]
    fn only_rollout_jsonl_is_indexed() {
        assert!(is_rollout(Path::new("rollout-2026-01.jsonl")));
        assert!(!is_rollout(Path::new("session_index.jsonl")));
        assert!(!is_rollout(Path::new("rollout-2026-01.json")));
    }

    #[test]
    fn all_supported_sources_are_visible_even_when_unavailable() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("meter.sqlite3");
        migrate(&path).unwrap();
        let names = discover_sources(&path)
            .unwrap()
            .into_iter()
            .map(|source| source.name)
            .collect::<Vec<_>>();
        for expected in ["Codex", "ZCode", "OpenCode", "DSH", "Claude", "EvoX"] {
            assert!(names.contains(&expected.to_string()));
        }
    }
}
