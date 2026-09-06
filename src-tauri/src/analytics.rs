use crate::db::{self, AppResult};
use crate::models::{
    AnalyticsFilters, MetricPeriod, MetricSeriesPoint, MetricSummary, ModelEffortStat,
    PricingCatalogStatus, PricingCoverage, PricingRate, UsageTokens,
};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::BTreeMap;
use std::path::Path;

const CATALOG_VERSION: &str = "2026-09-06.1";
const VERIFIED_AT: &str = "2026-09-06";

#[derive(Clone, Copy)]
struct RateDef {
    vendor: &'static str,
    model: &'static str,
    aliases: &'static [&'static str],
    input: i64,
    cached_read: Option<i64>,
    cached_write: Option<i64>,
    output: i64,
    effective_from: &'static str,
    effective_to: Option<&'static str>,
    source_url: &'static str,
}

// Rates use pico-USD per token so sub-nano prices remain exact. Observation totals
// are rounded once to integer nano-USD after all non-overlapping buckets are added.
const RATES: &[RateDef] = &[
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.6-sol",
        aliases: &["gpt-5.6"],
        input: 4_000_000,
        cached_read: Some(400_000),
        cached_write: Some(5_000_000),
        output: 20_000_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.6-sol",
    },
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.6-terra",
        aliases: &[],
        input: 2_000_000,
        cached_read: Some(200_000),
        cached_write: Some(2_500_000),
        output: 12_000_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.6-terra",
    },
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.6-luna",
        aliases: &[],
        input: 200_000,
        cached_read: Some(20_000),
        cached_write: Some(250_000),
        output: 1_200_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.6-luna",
    },
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.5",
        aliases: &[],
        input: 5_000_000,
        cached_read: Some(500_000),
        cached_write: None,
        output: 30_000_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.5",
    },
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.3-codex",
        aliases: &[],
        input: 1_750_000,
        cached_read: Some(175_000),
        cached_write: None,
        output: 14_000_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.3-codex",
    },
    RateDef {
        vendor: "OpenAI",
        model: "gpt-5.2-codex",
        aliases: &["gpt-5.2"],
        input: 1_750_000,
        cached_read: Some(175_000),
        cached_write: None,
        output: 14_000_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.2-codex",
    },
    RateDef {
        vendor: "MiniMax",
        model: "MiniMax-M2.7",
        aliases: &[],
        input: 300_000,
        cached_read: Some(60_000),
        cached_write: Some(375_000),
        output: 1_200_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://platform.minimax.io/docs/guides/pricing-paygo",
    },
    RateDef {
        vendor: "MiniMax",
        model: "MiniMax-M2.7-highspeed",
        aliases: &[],
        input: 600_000,
        cached_read: Some(60_000),
        cached_write: Some(375_000),
        output: 2_400_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://platform.minimax.io/docs/guides/pricing-paygo",
    },
    RateDef {
        vendor: "MiniMax",
        model: "MiniMax-M2.5",
        aliases: &[],
        input: 300_000,
        cached_read: Some(30_000),
        cached_write: Some(375_000),
        output: 1_200_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://platform.minimax.io/docs/guides/pricing-paygo",
    },
    RateDef {
        vendor: "MiniMax",
        model: "MiniMax-M2.5-highspeed",
        aliases: &[],
        input: 600_000,
        cached_read: Some(30_000),
        cached_write: Some(375_000),
        output: 2_400_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://platform.minimax.io/docs/guides/pricing-paygo",
    },
    RateDef {
        vendor: "MiniMax",
        model: "MiniMax-M2",
        aliases: &[],
        input: 300_000,
        cached_read: Some(30_000),
        cached_write: Some(375_000),
        output: 1_200_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://platform.minimax.io/docs/guides/pricing-paygo",
    },
    RateDef {
        vendor: "DeepSeek",
        model: "deepseek-v4-flash",
        aliases: &[
            "deepseek-v4-flash-free",
            "deepseek-chat",
            "deepseek-reasoner",
        ],
        input: 140_000,
        cached_read: Some(2_800),
        cached_write: None,
        output: 280_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://api-docs.deepseek.com/quick_start/pricing",
    },
    RateDef {
        vendor: "DeepSeek",
        model: "deepseek-v4-pro",
        aliases: &[],
        input: 435_000,
        cached_read: Some(3_625),
        cached_write: None,
        output: 870_000,
        effective_from: "2026-01-01",
        effective_to: None,
        source_url: "https://api-docs.deepseek.com/quick_start/pricing",
    },
];

#[derive(Clone)]
struct Observation {
    source_id: i64,
    source_name: String,
    provider: String,
    model: String,
    effort: String,
    completed_at: i64,
    duration_ms: Option<i64>,
    ttft_ms: Option<i64>,
    tokens: UsageTokens,
    cost: i64,
    priced_tokens: i64,
    pricing_status: String,
    updated_at: String,
}

pub fn migrate(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_observations (
            source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            external_id TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'unknown',
            model TEXT NOT NULL DEFAULT 'unknown',
            reasoning_effort TEXT NOT NULL DEFAULT 'default',
            started_at_ms INTEGER NOT NULL,
            completed_at_ms INTEGER,
            duration_ms INTEGER,
            ttft_ms INTEGER,
            status TEXT NOT NULL,
            uncached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cached_read_tokens INTEGER NOT NULL DEFAULT 0,
            cached_write_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_nano_usd INTEGER NOT NULL DEFAULT 0,
            priced_tokens INTEGER NOT NULL DEFAULT 0,
            pricing_status TEXT NOT NULL DEFAULT 'unpriced',
            pricing_rate_key TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(source_id, external_id)
         );
         CREATE INDEX IF NOT EXISTS idx_observations_completed ON usage_observations(completed_at_ms);
         CREATE INDEX IF NOT EXISTS idx_observations_dimensions ON usage_observations(source_id, model, reasoning_effort);
         CREATE TABLE IF NOT EXISTS source_sync_cursors (
            source_id INTEGER PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
            updated_ms INTEGER NOT NULL DEFAULT 0,
            external_id TEXT NOT NULL DEFAULT ''
         );",
    )
    .map_err(db::to_error)?;
    sync_codex_turns(conn)?;
    reprice_conn(conn)
}

pub fn sync_all_sources(path: &Path) -> AppResult<()> {
    let conn = db::open(path)?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(db::to_error)?;
    let result = (|| {
        sync_codex_turns(&conn)?;
        sync_zcode(&conn)?;
        sync_opencode(&conn)?;
        reprice_conn(&conn)
    })();
    if result.is_ok() {
        conn.execute_batch("COMMIT").map_err(db::to_error)?;
    } else {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

fn sync_codex_turns(conn: &Connection) -> AppResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT t.source_id, t.turn_id, t.model, t.reasoning_effort, t.started_at,
                    t.completed_at, t.duration_ms, t.ttft_ms, t.status,
                    t.input_tokens, t.cached_input_tokens, t.output_tokens,
                    t.reasoning_tokens, t.updated_at
             FROM turns t JOIN sources s ON s.id=t.source_id
             WHERE s.source_kind='codex_jsonl'",
        )
        .map_err(db::to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, String>(13)?,
            ))
        })
        .map_err(db::to_error)?;
    for row in rows {
        let (
            source_id,
            id,
            model,
            effort,
            started,
            completed,
            duration,
            ttft,
            status,
            input,
            cached,
            output,
            reasoning,
            updated,
        ) = row.map_err(db::to_error)?;
        let started_ms = parse_iso_ms(&started).unwrap_or(0);
        let completed_ms = completed.as_deref().and_then(parse_iso_ms);
        upsert_observation(
            conn,
            source_id,
            &format!("turn:{id}"),
            infer_vendor(&model),
            &model,
            &normalize_effort(effort.as_deref()),
            started_ms,
            completed_ms,
            duration,
            ttft,
            &status,
            (input - cached).max(0),
            cached.max(0),
            0,
            output.max(0),
            reasoning.max(0),
            &updated,
        )?;
    }
    Ok(())
}

fn readonly(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db::to_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(db::to_error)?;
    conn.pragma_update(None, "query_only", true)
        .map_err(db::to_error)?;
    Ok(conn)
}

fn source(conn: &Connection, kind: &str) -> AppResult<Option<(i64, String)>> {
    conn.query_row(
        "SELECT id, root_path FROM sources WHERE source_kind=?1 AND enabled=1 LIMIT 1",
        [kind],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(db::to_error)
}

fn sync_zcode(conn: &Connection) -> AppResult<()> {
    let Some((source_id, path)) = source(conn, "zcode_sqlite")? else {
        return Ok(());
    };
    if !Path::new(&path).is_file() {
        return Ok(());
    }
    let remote = readonly(Path::new(&path))?;
    let overlap_start = conn.query_row(
        "SELECT COALESCE(MAX(started_at_ms) - 86400000, 0) FROM usage_observations WHERE source_id=?1",
        [source_id],
        |row| row.get::<_, i64>(0),
    ).map_err(db::to_error)?;
    let mut stmt = remote
        .prepare(
            "SELECT id, provider_id, model_id, variant, status, started_at, completed_at,
                duration_ms, time_to_first_token_ms, input_tokens, output_tokens,
                reasoning_tokens, cache_creation_input_tokens, cache_read_input_tokens
         FROM model_usage WHERE started_at>=?1",
        )
        .map_err(db::to_error)?;
    let rows = stmt
        .query_map([overlap_start], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
            ))
        })
        .map_err(db::to_error)?;
    for row in rows {
        let (
            id,
            provider,
            model,
            effort,
            status,
            started,
            completed,
            duration,
            ttft,
            input,
            output,
            reasoning,
            cache_write,
            cache_read,
        ) = row.map_err(db::to_error)?;
        let updated = Utc
            .timestamp_millis_opt(completed.unwrap_or(started))
            .single()
            .unwrap_or_else(Utc::now)
            .to_rfc3339();
        upsert_observation(
            conn,
            source_id,
            &id,
            &provider,
            &model,
            &normalize_effort(effort.as_deref()),
            started,
            completed,
            duration,
            ttft,
            &status,
            (input - cache_read - cache_write).max(0),
            cache_read.max(0),
            cache_write.max(0),
            output.max(0),
            reasoning.max(0),
            &updated,
        )?;
    }
    Ok(())
}

fn sync_opencode(conn: &Connection) -> AppResult<()> {
    let Some((source_id, path)) = source(conn, "opencode_sqlite")? else {
        return Ok(());
    };
    if !Path::new(&path).is_file() {
        return Ok(());
    }
    let remote = readonly(Path::new(&path))?;
    let (cursor_ms, cursor_id) = conn
        .query_row(
            "SELECT updated_ms,external_id FROM source_sync_cursors WHERE source_id=?1",
            [source_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(db::to_error)?
        .unwrap_or((0, String::new()));
    let mut stmt = remote.prepare(
        "SELECT m.id, m.time_updated, json_extract(m.data,'$.providerID'),
                json_extract(m.data,'$.modelID'), json_extract(m.data,'$.variant'),
                json_extract(m.data,'$.time.created'), json_extract(m.data,'$.time.completed'),
                json_extract(m.data,'$.tokens.input'), json_extract(m.data,'$.tokens.cache.read'),
                json_extract(m.data,'$.tokens.cache.write'), json_extract(m.data,'$.tokens.output'),
                json_extract(m.data,'$.tokens.reasoning'),
                (SELECT min(json_extract(p.data,'$.time.start')) FROM part p WHERE p.message_id=m.id)
         FROM message m WHERE json_extract(m.data,'$.role')='assistant'
           AND (m.time_updated>?1 OR (m.time_updated=?1 AND m.id>?2))
         ORDER BY m.time_updated,m.id"
    ).map_err(db::to_error)?;
    let rows = stmt
        .query_map(params![cursor_ms, cursor_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<i64>>(12)?,
            ))
        })
        .map_err(db::to_error)?;
    let mut latest = (cursor_ms, cursor_id);
    for row in rows {
        let (
            id,
            updated_ms,
            provider,
            model,
            effort,
            started,
            completed,
            input,
            cache_read,
            cache_write,
            visible_output,
            reasoning,
            first,
        ) = row.map_err(db::to_error)?;
        let started = started.unwrap_or(updated_ms);
        let duration = completed.map(|end| (end - started).max(0));
        let ttft = first
            .filter(|value| *value >= started)
            .map(|value| value - started);
        let output = visible_output.unwrap_or(0).max(0) + reasoning.unwrap_or(0).max(0);
        let updated = Utc
            .timestamp_millis_opt(updated_ms)
            .single()
            .unwrap_or_else(Utc::now)
            .to_rfc3339();
        upsert_observation(
            conn,
            source_id,
            &id,
            provider.as_deref().unwrap_or("unknown"),
            model.as_deref().unwrap_or("unknown"),
            &normalize_effort(effort.as_deref()),
            started,
            completed,
            duration,
            ttft,
            if completed.is_some() {
                "completed"
            } else {
                "running"
            },
            input.unwrap_or(0).max(0),
            cache_read.unwrap_or(0).max(0),
            cache_write.unwrap_or(0).max(0),
            output,
            reasoning.unwrap_or(0).max(0),
            &updated,
        )?;
        latest = (updated_ms, id);
    }
    conn.execute(
        "INSERT INTO source_sync_cursors(source_id,updated_ms,external_id) VALUES (?1,?2,?3)
         ON CONFLICT(source_id) DO UPDATE SET updated_ms=excluded.updated_ms,external_id=excluded.external_id",
        params![source_id, latest.0, latest.1],
    ).map_err(db::to_error)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn upsert_observation(
    conn: &Connection,
    source_id: i64,
    external_id: &str,
    provider: &str,
    model: &str,
    effort: &str,
    started: i64,
    completed: Option<i64>,
    duration: Option<i64>,
    ttft: Option<i64>,
    status: &str,
    uncached: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    updated: &str,
) -> AppResult<()> {
    let total = uncached
        .saturating_add(cache_read)
        .saturating_add(cache_write)
        .saturating_add(output);
    conn.execute(
        "INSERT INTO usage_observations(source_id,external_id,provider,model,reasoning_effort,started_at_ms,completed_at_ms,duration_ms,ttft_ms,status,uncached_input_tokens,cached_read_tokens,cached_write_tokens,output_tokens,reasoning_tokens,total_tokens,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
         ON CONFLICT(source_id,external_id) DO UPDATE SET provider=excluded.provider,model=excluded.model,reasoning_effort=excluded.reasoning_effort,started_at_ms=excluded.started_at_ms,completed_at_ms=excluded.completed_at_ms,duration_ms=excluded.duration_ms,ttft_ms=excluded.ttft_ms,status=excluded.status,uncached_input_tokens=excluded.uncached_input_tokens,cached_read_tokens=excluded.cached_read_tokens,cached_write_tokens=excluded.cached_write_tokens,output_tokens=excluded.output_tokens,reasoning_tokens=excluded.reasoning_tokens,total_tokens=excluded.total_tokens,updated_at=excluded.updated_at",
        params![source_id, external_id, provider, model, effort, started, completed, duration, ttft, status, uncached, cache_read, cache_write, output, reasoning, total, updated],
    ).map_err(db::to_error)?;
    Ok(())
}

fn normalize_effort(value: Option<&str>) -> String {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra") => {
            value.unwrap().to_ascii_lowercase()
        }
        None | Some("" | "default") => "default".into(),
        _ => "unknown".into(),
    }
}

fn infer_vendor(model: &str) -> &str {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("gpt-") || lower.starts_with("o3") || lower.starts_with("o4") {
        "OpenAI"
    } else if lower.starts_with("minimax") {
        "MiniMax"
    } else if lower.starts_with("deepseek") {
        "DeepSeek"
    } else {
        "unknown"
    }
}

fn is_local(provider: &str, model: &str) -> bool {
    let p = provider.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();
    p.starts_with("omlx")
        || ["qwen", "ornith", "macaron", "gemma"]
            .iter()
            .any(|prefix| m.starts_with(prefix))
}

fn rate_for(model: &str, date: &str) -> Option<&'static RateDef> {
    RATES
        .iter()
        .filter(|rate| rate.effective_from <= date)
        .filter(|rate| rate.effective_to.is_none_or(|end| date < end))
        .filter(|rate| {
            rate.model.eq_ignore_ascii_case(model)
                || rate
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(model))
        })
        .max_by_key(|rate| rate.effective_from)
}

fn reprice_conn(conn: &Connection) -> AppResult<()> {
    let mut stmt = conn.prepare("SELECT source_id,external_id,provider,model,started_at_ms,uncached_input_tokens,cached_read_tokens,cached_write_tokens,output_tokens,total_tokens FROM usage_observations").map_err(db::to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(db::to_error)?;
    let values = rows.collect::<Result<Vec<_>, _>>().map_err(db::to_error)?;
    for (
        source_id,
        id,
        provider,
        model,
        started,
        uncached,
        cache_read,
        cache_write,
        output,
        total,
    ) in values
    {
        let date = Local
            .timestamp_millis_opt(started)
            .single()
            .map(|v| v.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "1970-01-01".into());
        let (cost, priced, status, key) = if is_local(&provider, &model) {
            (0, total, "free", Some("local".to_string()))
        } else if let Some(rate) = rate_for(&model, &date) {
            let mut cost_pico =
                uncached.saturating_mul(rate.input) + output.saturating_mul(rate.output);
            let mut priced = uncached + output;
            if let Some(value) = rate.cached_read {
                cost_pico = cost_pico.saturating_add(cache_read.saturating_mul(value));
                priced += cache_read;
            }
            if let Some(value) = rate.cached_write {
                cost_pico = cost_pico.saturating_add(cache_write.saturating_mul(value));
                priced += cache_write;
            }
            let cost = cost_pico.saturating_add(500) / 1_000;
            let status = if priced >= total { "priced" } else { "partial" };
            (
                cost,
                priced,
                status,
                Some(format!("{}:{}", rate.vendor, rate.model)),
            )
        } else {
            (0, 0, "unpriced", None)
        };
        conn.execute("UPDATE usage_observations SET estimated_cost_nano_usd=?1,priced_tokens=?2,pricing_status=?3,pricing_rate_key=?4 WHERE source_id=?5 AND external_id=?6", params![cost,priced,status,key,source_id,id]).map_err(db::to_error)?;
    }
    Ok(())
}

pub fn reprice_usage(path: &Path) -> AppResult<PricingCatalogStatus> {
    let conn = db::open(path)?;
    reprice_conn(&conn)?;
    pricing_status_conn(&conn)
}

pub fn pricing_catalog_status(path: &Path) -> AppResult<PricingCatalogStatus> {
    pricing_status_conn(&db::open(path)?)
}

fn pricing_status_conn(conn: &Connection) -> AppResult<PricingCatalogStatus> {
    let (priced,total) = conn.query_row("SELECT count(*) FILTER (WHERE pricing_status IN ('priced','free')),count(*) FROM usage_observations WHERE status='completed'", [], |row| Ok((row.get(0)?,row.get(1)?))).map_err(db::to_error)?;
    Ok(PricingCatalogStatus {
        version: CATALOG_VERSION.into(),
        currency: "USD".into(),
        verified_at: VERIFIED_AT.into(),
        rates: RATES
            .iter()
            .map(|r| PricingRate {
                vendor: r.vendor.into(),
                model: r.model.into(),
                aliases: r.aliases.iter().map(|v| (*v).into()).collect(),
                currency: "USD".into(),
                input_usd_per_million: rate_string(r.input),
                cached_read_usd_per_million: r.cached_read.map(rate_string),
                cached_write_usd_per_million: r.cached_write.map(rate_string),
                output_usd_per_million: rate_string(r.output),
                effective_from: r.effective_from.into(),
                effective_to: r.effective_to.map(Into::into),
                source_url: r.source_url.into(),
                verified_at: VERIFIED_AT.into(),
            })
            .collect(),
        priced_observations: priced,
        total_observations: total,
    })
}

fn rate_string(pico_per_token: i64) -> String {
    let value = pico_per_token as f64 / 1_000_000.0;
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.6}").trim_end_matches('0').to_string()
    }
}

fn parse_iso_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.timestamp_millis())
}

fn period_start(period: MetricPeriod) -> Option<i64> {
    if period == MetricPeriod::Realtime {
        return None;
    }
    let today = Local::now().date_naive();
    let date = match period {
        MetricPeriod::Today => today,
        MetricPeriod::Week => today - Duration::days(today.weekday().num_days_from_monday() as i64),
        MetricPeriod::Month => {
            NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today)
        }
        MetricPeriod::Year => NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
        MetricPeriod::Realtime => return None,
    };
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|v| v.timestamp_millis())
}

fn load(path: &Path, filters: &AnalyticsFilters) -> AppResult<Vec<Observation>> {
    let conn = db::open(path)?;
    let mut sql = "SELECT o.source_id,s.name,o.provider,o.model,o.reasoning_effort,o.completed_at_ms,o.duration_ms,o.ttft_ms,o.uncached_input_tokens,o.cached_read_tokens,o.cached_write_tokens,o.output_tokens,o.reasoning_tokens,o.total_tokens,o.estimated_cost_nano_usd,o.priced_tokens,o.pricing_status,o.updated_at FROM usage_observations o JOIN sources s ON s.id=o.source_id WHERE s.enabled=1 AND o.status='completed' AND o.completed_at_ms IS NOT NULL".to_string();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(start) = period_start(filters.period) {
        sql.push_str(" AND o.completed_at_ms>=?");
        values.push(Box::new(start));
    }
    if let Some(source_id) = filters.source_id {
        sql.push_str(" AND o.source_id=?");
        values.push(Box::new(source_id));
    }
    if let Some(model) = filters.model.as_ref().filter(|v| !v.is_empty()) {
        sql.push_str(" AND o.model=?");
        values.push(Box::new(model.clone()));
    }
    if let Some(effort) = filters.reasoning_effort.as_ref().filter(|v| !v.is_empty()) {
        sql.push_str(" AND o.reasoning_effort=?");
        values.push(Box::new(effort.clone()));
    }
    sql.push_str(" ORDER BY o.completed_at_ms DESC");
    if filters.period == MetricPeriod::Realtime {
        sql.push_str(" LIMIT 10");
    }
    let refs = values.iter().map(|v| v.as_ref()).collect::<Vec<_>>();
    let mut stmt = conn.prepare(&sql).map_err(db::to_error)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(refs), |row| {
            Ok(Observation {
                source_id: row.get(0)?,
                source_name: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                effort: row.get(4)?,
                completed_at: row.get(5)?,
                duration_ms: row.get(6)?,
                ttft_ms: row.get(7)?,
                tokens: UsageTokens {
                    uncached_input: row.get(8)?,
                    cached_read: row.get(9)?,
                    cached_write: row.get(10)?,
                    output: row.get(11)?,
                    reasoning: row.get(12)?,
                    total: row.get(13)?,
                },
                cost: row.get(14)?,
                priced_tokens: row.get(15)?,
                pricing_status: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })
        .map_err(db::to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db::to_error)
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn tps(row: &Observation) -> Option<f64> {
    let duration = row.duration_ms?;
    let ttft = row.ttft_ms?;
    (row.tokens.output > 0 && duration > ttft)
        .then(|| row.tokens.output as f64 / ((duration - ttft) as f64 / 1000.0))
}

fn summarize(period: MetricPeriod, rows: &[Observation]) -> MetricSummary {
    let mut tokens = UsageTokens::default();
    for row in rows {
        tokens.uncached_input += row.tokens.uncached_input;
        tokens.cached_read += row.tokens.cached_read;
        tokens.cached_write += row.tokens.cached_write;
        tokens.output += row.tokens.output;
        tokens.reasoning += row.tokens.reasoning;
        tokens.total += row.tokens.total;
    }
    let priced_tokens = rows.iter().map(|r| r.priced_tokens).sum::<i64>();
    let priced_observations = rows
        .iter()
        .filter(|r| r.pricing_status == "priced" || r.pricing_status == "free")
        .count() as i64;
    let ratio = if tokens.total > 0 {
        priced_tokens as f64 / tokens.total as f64
    } else {
        1.0
    };
    MetricSummary {
        period,
        observation_count: rows.len() as i64,
        average_ttft_ms: average(rows.iter().filter_map(|r| r.ttft_ms.map(|v| v as f64))),
        average_effective_tps: average(rows.iter().filter_map(tps)),
        tokens,
        estimated_cost_nano_usd: rows.iter().map(|r| r.cost).sum(),
        pricing: PricingCoverage {
            priced_observations,
            total_observations: rows.len() as i64,
            priced_tokens,
            total_tokens: tokens.total,
            ratio,
            complete: priced_tokens >= tokens.total,
        },
        last_updated_at: rows.iter().map(|r| r.updated_at.clone()).max(),
    }
}

pub fn query_metric_summary(path: &Path, filters: AnalyticsFilters) -> AppResult<MetricSummary> {
    let rows = load(path, &filters)?;
    Ok(summarize(filters.period, &rows))
}

pub fn query_model_effort_stats(
    path: &Path,
    filters: AnalyticsFilters,
) -> AppResult<Vec<ModelEffortStat>> {
    let rows = load(path, &filters)?;
    let mut groups: BTreeMap<(i64, String, String, String, String), Vec<Observation>> =
        BTreeMap::new();
    for row in rows {
        groups
            .entry((
                row.source_id,
                row.source_name.clone(),
                row.provider.clone(),
                row.model.clone(),
                row.effort.clone(),
            ))
            .or_default()
            .push(row);
    }
    let mut result = groups
        .into_iter()
        .map(
            |((source_id, source_name, provider, model, effort), rows)| {
                let summary = summarize(filters.period, &rows);
                ModelEffortStat {
                    source_id,
                    source_name,
                    provider,
                    model,
                    reasoning_effort: effort,
                    observation_count: summary.observation_count,
                    average_ttft_ms: summary.average_ttft_ms,
                    average_effective_tps: summary.average_effective_tps,
                    tokens: summary.tokens,
                    estimated_cost_nano_usd: summary.estimated_cost_nano_usd,
                    pricing: summary.pricing,
                }
            },
        )
        .collect::<Vec<_>>();
    result.sort_by_key(|row| std::cmp::Reverse(row.tokens.total));
    Ok(result)
}

pub fn query_metric_series(
    path: &Path,
    filters: AnalyticsFilters,
) -> AppResult<Vec<MetricSeriesPoint>> {
    let rows = load(path, &filters)?;
    let mut groups: BTreeMap<String, (String, Vec<Observation>)> = BTreeMap::new();
    for row in rows.into_iter().rev() {
        let local = Local.timestamp_millis_opt(row.completed_at).single();
        let (bucket, label) = match (filters.period, local) {
            (MetricPeriod::Realtime, Some(v)) => (
                format!("{}-{}", v.format("%H:%M:%S"), row.model),
                v.format("%H:%M").to_string(),
            ),
            (MetricPeriod::Today, Some(v)) => (
                v.format("%Y-%m-%d-%H").to_string(),
                v.format("%H:00").to_string(),
            ),
            (MetricPeriod::Week | MetricPeriod::Month, Some(v)) => (
                v.format("%Y-%m-%d").to_string(),
                v.format("%m-%d").to_string(),
            ),
            (MetricPeriod::Year, Some(v)) => {
                (v.format("%Y-%m").to_string(), v.format("%Y-%m").to_string())
            }
            (_, None) => (row.completed_at.to_string(), "--".into()),
        };
        groups
            .entry(bucket)
            .or_insert_with(|| (label, Vec::new()))
            .1
            .push(row);
    }
    Ok(groups
        .into_iter()
        .map(|(bucket, (label, rows))| {
            let s = summarize(filters.period, &rows);
            MetricSeriesPoint {
                bucket,
                label,
                observation_count: s.observation_count,
                average_ttft_ms: s.average_ttft_ms,
                average_effective_tps: s.average_effective_tps,
                total_tokens: s.tokens.total,
                estimated_cost_nano_usd: s.estimated_cost_nano_usd,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    fn empty_meter() -> (TempDir, std::path::PathBuf) {
        let temp = tempdir().unwrap();
        let path = temp.path().join("meter.sqlite3");
        db::migrate(&path).unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("DELETE FROM sources", []).unwrap();
        (temp, path)
    }

    #[test]
    fn normalizes_reasoning_effort() {
        assert_eq!(normalize_effort(Some("HIGH")), "high");
        assert_eq!(normalize_effort(Some("enabled")), "unknown");
        assert_eq!(normalize_effort(None), "default");
    }

    #[test]
    fn prices_non_overlapping_buckets() {
        let rate = rate_for("gpt-5.6-luna", "2026-09-06").unwrap();
        assert_eq!(
            10 * rate.input
                + 20 * rate.cached_read.unwrap()
                + 3 * rate.cached_write.unwrap()
                + 5 * rate.output,
            10 * 200_000 + 20 * 20_000 + 3 * 250_000 + 5 * 1_200_000
        );
    }

    #[test]
    fn preserves_sub_nano_official_rates() {
        let rate = rate_for("deepseek-v4-flash", "2026-09-06").unwrap();
        assert_eq!(rate.cached_read, Some(2_800));
        assert_eq!(rate_string(rate.cached_read.unwrap()), "0.0028");
    }

    #[test]
    fn resolves_aliases_only_inside_the_price_window() {
        assert!(rate_for("gpt-5.6", "2025-12-31").is_none());
        assert_eq!(
            rate_for("gpt-5.6", "2026-01-01").unwrap().model,
            "gpt-5.6-sol"
        );
    }

    #[test]
    fn partial_pricing_keeps_known_cost_and_reports_coverage() {
        let rows = [
            Observation {
                source_id: 1,
                source_name: "Fixture".into(),
                provider: "OpenAI".into(),
                model: "gpt-5.6-sol".into(),
                effort: "high".into(),
                completed_at: 1,
                duration_ms: Some(2_000),
                ttft_ms: Some(1_000),
                tokens: UsageTokens {
                    uncached_input: 50,
                    output: 50,
                    total: 100,
                    ..Default::default()
                },
                cost: 1_000,
                priced_tokens: 100,
                pricing_status: "priced".into(),
                updated_at: "2026-09-06T00:00:00Z".into(),
            },
            Observation {
                source_id: 1,
                source_name: "Fixture".into(),
                provider: "unknown".into(),
                model: "unknown".into(),
                effort: "unknown".into(),
                completed_at: 2,
                duration_ms: None,
                ttft_ms: None,
                tokens: UsageTokens {
                    uncached_input: 100,
                    total: 100,
                    ..Default::default()
                },
                cost: 0,
                priced_tokens: 0,
                pricing_status: "unpriced".into(),
                updated_at: "2026-09-06T00:00:01Z".into(),
            },
        ];
        let summary = summarize(MetricPeriod::Today, &rows);
        assert_eq!(summary.estimated_cost_nano_usd, 1_000);
        assert_eq!(summary.pricing.ratio, 0.5);
        assert!(!summary.pricing.complete);
    }

    #[test]
    fn recognizes_local_models_as_free() {
        assert!(is_local("omlx2", "anything"));
        assert!(is_local("uuid", "Qwen3.8-27B-MLX-4bit"));
    }

    #[test]
    fn realtime_summary_uses_only_latest_ten_observations() {
        let (_temp, path) = empty_meter();
        let conn = db::open(&path).unwrap();
        conn.execute(
            "INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (100,'Fixture','/fixture','fixture',1)",
            [],
        ).unwrap();
        let now = Utc::now().timestamp_millis();
        for index in 0..12_i64 {
            upsert_observation(
                &conn,
                100,
                &format!("row-{index}"),
                "omlx",
                "Qwen-fixture",
                "default",
                now - index * 1_000,
                Some(now - index * 1_000),
                Some(2_000),
                Some(1_000),
                "completed",
                10,
                0,
                0,
                10,
                0,
                &Utc::now().to_rfc3339(),
            )
            .unwrap();
        }
        drop(conn);
        let summary = query_metric_summary(
            &path,
            AnalyticsFilters {
                period: MetricPeriod::Realtime,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(summary.observation_count, 10);
        assert_eq!(summary.tokens.total, 200);
        assert_eq!(summary.average_effective_tps, Some(10.0));
    }

    #[test]
    fn zcode_wal_sync_updates_running_record_without_duplication() {
        let (temp, path) = empty_meter();
        let remote_path = temp.path().join("zcode.sqlite");
        let remote = Connection::open(&remote_path).unwrap();
        remote.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE model_usage(id TEXT PRIMARY KEY,provider_id TEXT NOT NULL,model_id TEXT NOT NULL,variant TEXT,status TEXT NOT NULL,started_at INTEGER NOT NULL,completed_at INTEGER,duration_ms INTEGER,time_to_first_token_ms INTEGER,input_tokens INTEGER NOT NULL,output_tokens INTEGER NOT NULL,reasoning_tokens INTEGER NOT NULL,cache_creation_input_tokens INTEGER NOT NULL,cache_read_input_tokens INTEGER NOT NULL);").unwrap();
        remote.execute("INSERT INTO model_usage VALUES ('call-1','omlx-local','Qwen-fixture','enabled','running',1000,NULL,NULL,120,100,20,5,10,30)", []).unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (101,'ZCode',?1,'zcode_sqlite',1)", [remote_path.to_string_lossy().as_ref()]).unwrap();
        drop(conn);
        sync_all_sources(&path).unwrap();
        remote.execute("UPDATE model_usage SET status='completed',completed_at=3000,duration_ms=2000,output_tokens=25 WHERE id='call-1'", []).unwrap();
        sync_all_sources(&path).unwrap();
        let conn = db::open(&path).unwrap();
        let row: (i64,String,i64,i64,i64) = conn.query_row("SELECT count(*),status,uncached_input_tokens,cached_read_tokens,total_tokens FROM usage_observations WHERE source_id=101", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(row, (1, "completed".into(), 60, 30, 125));
    }

    #[test]
    fn opencode_cursor_updates_existing_call_and_does_not_invent_ttft() {
        let (temp, path) = empty_meter();
        let remote_path = temp.path().join("opencode.sqlite");
        let remote = Connection::open(&remote_path).unwrap();
        remote.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE message(id TEXT PRIMARY KEY,time_updated INTEGER NOT NULL,data TEXT NOT NULL); CREATE TABLE part(id TEXT PRIMARY KEY,message_id TEXT NOT NULL,data TEXT NOT NULL);").unwrap();
        let running = r#"{"role":"assistant","providerID":"custom","modelID":"muse-pro","variant":"disabled","time":{"created":1000},"tokens":{"input":10,"cache":{"read":3,"write":2},"output":4,"reasoning":1}}"#;
        remote
            .execute("INSERT INTO message VALUES ('msg-1',1000,?1)", [running])
            .unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (102,'OpenCode',?1,'opencode_sqlite',1)", [remote_path.to_string_lossy().as_ref()]).unwrap();
        drop(conn);
        sync_all_sources(&path).unwrap();
        let completed = r#"{"role":"assistant","providerID":"custom","modelID":"muse-pro","variant":"disabled","time":{"created":1000,"completed":3000},"tokens":{"input":10,"cache":{"read":3,"write":2},"output":4,"reasoning":1}}"#;
        remote
            .execute(
                "UPDATE message SET time_updated=3000,data=?1 WHERE id='msg-1'",
                [completed],
            )
            .unwrap();
        sync_all_sources(&path).unwrap();
        let conn = db::open(&path).unwrap();
        let row: (i64,Option<i64>,i64,i64,String) = conn.query_row("SELECT count(*),ttft_ms,output_tokens,total_tokens,reasoning_effort FROM usage_observations WHERE source_id=102", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(row, (1, None, 5, 20, "unknown".into()));
        let cursor: (i64, String) = conn
            .query_row(
                "SELECT updated_ms,external_id FROM source_sync_cursors WHERE source_id=102",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(cursor, (3000, "msg-1".into()));
    }
}
