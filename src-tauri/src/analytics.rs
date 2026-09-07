use crate::db::{self, AppResult};
use crate::models::{
    AnalyticsFilters, DataIntegrityStatus, MetricPeriod, MetricSeriesPoint, MetricSummary,
    ModelEffortStat, PricingCatalogStatus, PricingCoverage, PricingModel, PricingRate,
    PricingRateInput, SourceIntegrityStatus, UsageTokens,
};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use walkdir::WalkDir;

const CATALOG_VERSION: &str = "2026-09-06.2";
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
        model: "gpt-6-astra",
        aliases: &[],
        input: 10_000_000,
        cached_read: Some(1_000_000),
        cached_write: Some(12_500_000),
        output: 50_000_000,
        effective_from: "2026-09-07",
        effective_to: None,
        source_url: "https://developers.openai.com/api/docs/models/gpt-6-astra",
    },
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
    RateDef {
        vendor: "Meta",
        model: "muse-spark-1.3",
        aliases: &[
            "muse-spark-1.3-contributor",
            "muse-spark-1.3-contributor-free",
        ],
        input: 100_000,
        cached_read: Some(2_000),
        cached_write: None,
        output: 200_000,
        effective_from: "2026-09-03",
        effective_to: None,
        source_url: "https://vercel.com/changelog/muse-spark-1-3-now-available-on-ai-gateway",
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
         );
         CREATE TABLE IF NOT EXISTS model_call_observations (
            source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            external_id TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'unknown',
            model TEXT NOT NULL DEFAULT 'unknown',
            reasoning_effort TEXT NOT NULL DEFAULT 'default',
            occurred_at_ms INTEGER NOT NULL,
            uncached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cached_read_tokens INTEGER NOT NULL DEFAULT 0,
            cached_write_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_nano_usd INTEGER NOT NULL DEFAULT 0,
            priced_tokens INTEGER NOT NULL DEFAULT 0,
            pricing_status TEXT NOT NULL DEFAULT 'unpriced',
            pricing_rate_id INTEGER,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(source_id, external_id)
         );
         CREATE INDEX IF NOT EXISTS idx_call_observations_time ON model_call_observations(occurred_at_ms);
         CREATE INDEX IF NOT EXISTS idx_call_observations_dimensions ON model_call_observations(source_id, model, reasoning_effort);
         CREATE TABLE IF NOT EXISTS performance_observations (
            source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            external_id TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'unknown',
            model TEXT NOT NULL DEFAULT 'unknown',
            reasoning_effort TEXT NOT NULL DEFAULT 'default',
            occurred_at_ms INTEGER NOT NULL,
            duration_ms INTEGER,
            ttft_ms INTEGER,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(source_id, external_id)
         );
         CREATE INDEX IF NOT EXISTS idx_performance_observations_time ON performance_observations(occurred_at_ms);
         CREATE TABLE IF NOT EXISTS pricing_rates (
            id INTEGER PRIMARY KEY,
            vendor TEXT NOT NULL,
            model TEXT NOT NULL COLLATE NOCASE,
            input_pico_per_token INTEGER NOT NULL,
            cached_read_pico_per_token INTEGER,
            cached_write_pico_per_token INTEGER,
            output_pico_per_token INTEGER NOT NULL,
            effective_from TEXT NOT NULL,
            effective_to TEXT,
            source_url TEXT NOT NULL DEFAULT '',
            verified_at TEXT NOT NULL,
            origin TEXT NOT NULL DEFAULT 'builtin',
            user_modified INTEGER NOT NULL DEFAULT 0,
            deleted_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(model, effective_from)
         );
         CREATE TABLE IF NOT EXISTS pricing_rate_aliases (
            rate_id INTEGER NOT NULL REFERENCES pricing_rates(id) ON DELETE CASCADE,
            alias TEXT NOT NULL COLLATE NOCASE,
            PRIMARY KEY(rate_id, alias)
         );
         CREATE TABLE IF NOT EXISTS pricing_seed_tombstones (
            model TEXT NOT NULL COLLATE NOCASE,
            effective_from TEXT NOT NULL,
            deleted_at TEXT NOT NULL,
            PRIMARY KEY(model, effective_from)
         );",
    )
    .map_err(db::to_error)?;
    seed_pricing_rates(conn)?;
    sync_codex_turns(conn)?;
    conn.execute_batch("SAVEPOINT analytics_v2")
        .map_err(db::to_error)?;
    let result = materialize_observations(conn).and_then(|_| reprice_conn(conn));
    if result.is_ok() {
        conn.execute_batch("RELEASE analytics_v2")
            .map_err(db::to_error)?;
    } else {
        let _ = conn.execute_batch("ROLLBACK TO analytics_v2; RELEASE analytics_v2");
    }
    result
}

pub fn sync_all_sources(path: &Path) -> AppResult<()> {
    let conn = db::open(path)?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(db::to_error)?;
    let result = (|| {
        sync_codex_turns(&conn)?;
        sync_zcode(&conn)?;
        sync_opencode(&conn)?;
        sync_dsh(&conn)?;
        sync_evox(&conn)?;
        materialize_observations(&conn)?;
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

fn seed_pricing_rates(conn: &Connection) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    for rate in RATES {
        let tombstoned = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pricing_seed_tombstones WHERE model=?1 AND effective_from=?2)",
                params![rate.model, rate.effective_from],
                |row| row.get::<_, bool>(0),
            )
            .map_err(db::to_error)?;
        if tombstoned {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO pricing_rates(vendor,model,input_pico_per_token,cached_read_pico_per_token,cached_write_pico_per_token,output_pico_per_token,effective_from,effective_to,source_url,verified_at,origin,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'builtin',?11,?11)",
            params![rate.vendor,rate.model,rate.input,rate.cached_read,rate.cached_write,rate.output,rate.effective_from,rate.effective_to,rate.source_url,VERIFIED_AT,now],
        ).map_err(db::to_error)?;
        let id = conn.query_row(
            "SELECT id FROM pricing_rates WHERE model=?1 AND effective_from=?2 AND deleted_at IS NULL",
            params![rate.model, rate.effective_from],
            |row| row.get::<_, i64>(0),
        ).optional().map_err(db::to_error)?;
        if let Some(id) = id {
            for alias in rate.aliases {
                conn.execute(
                    "INSERT OR IGNORE INTO pricing_rate_aliases(rate_id,alias) VALUES (?1,?2)",
                    params![id, alias],
                )
                .map_err(db::to_error)?;
            }
        }
    }
    Ok(())
}

fn materialize_observations(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM model_call_observations", [])
        .map_err(db::to_error)?;
    conn.execute("DELETE FROM performance_observations", [])
        .map_err(db::to_error)?;

    conn.execute(
        "INSERT INTO model_call_observations(source_id,external_id,provider,model,reasoning_effort,occurred_at_ms,uncached_input_tokens,cached_read_tokens,cached_write_tokens,output_tokens,reasoning_tokens,total_tokens,updated_at)
         SELECT o.source_id,o.external_id,o.provider,o.model,o.reasoning_effort,o.completed_at_ms,
                o.uncached_input_tokens,o.cached_read_tokens,o.cached_write_tokens,o.output_tokens,o.reasoning_tokens,o.total_tokens,o.updated_at
         FROM usage_observations o JOIN sources s ON s.id=o.source_id
         WHERE s.source_kind!='codex_jsonl' AND o.status='completed' AND o.completed_at_ms IS NOT NULL AND o.total_tokens>0",
        [],
    ).map_err(db::to_error)?;
    conn.execute(
        "INSERT INTO performance_observations(source_id,external_id,provider,model,reasoning_effort,occurred_at_ms,duration_ms,ttft_ms,output_tokens,updated_at)
         SELECT o.source_id,o.external_id,o.provider,o.model,o.reasoning_effort,o.completed_at_ms,o.duration_ms,o.ttft_ms,o.output_tokens,o.updated_at
         FROM usage_observations o JOIN sources s ON s.id=o.source_id
         WHERE s.source_kind!='codex_jsonl' AND o.status='completed' AND o.completed_at_ms IS NOT NULL AND o.duration_ms IS NOT NULL",
        [],
    ).map_err(db::to_error)?;

    let mut stmt = conn.prepare(
        "SELECT t.source_id,c.response_id,t.model,t.reasoning_effort,c.occurred_at,c.input_tokens,c.cached_input_tokens,c.output_tokens,c.reasoning_tokens,c.total_tokens,t.updated_at
         FROM model_calls c JOIN turns t ON t.turn_id=c.turn_id
         WHERE c.total_tokens>0 AND (
           c.call_kind='primary' OR (c.call_kind='legacy' AND NOT EXISTS (
             SELECT 1 FROM model_calls p WHERE p.turn_id=c.turn_id AND p.call_kind='primary'
           ))
         )",
    ).map_err(db::to_error)?;
    let calls = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, String>(10)?,
            ))
        })
        .map_err(db::to_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db::to_error)?;
    for (
        source_id,
        id,
        model,
        effort,
        occurred,
        input,
        cached,
        output,
        reasoning,
        total,
        updated,
    ) in calls
    {
        let occurred_ms = parse_iso_ms(&occurred).unwrap_or(0);
        conn.execute(
            "INSERT INTO model_call_observations(source_id,external_id,provider,model,reasoning_effort,occurred_at_ms,uncached_input_tokens,cached_read_tokens,cached_write_tokens,output_tokens,reasoning_tokens,total_tokens,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,?9,?10,?11,?12)",
            params![source_id,id,infer_vendor(&model),model,normalize_effort(effort.as_deref()),occurred_ms,(input-cached).max(0),cached.max(0),output.max(0),reasoning.max(0),total.max(0),updated],
        ).map_err(db::to_error)?;
    }
    conn.execute(
        "INSERT INTO performance_observations(source_id,external_id,provider,model,reasoning_effort,occurred_at_ms,duration_ms,ttft_ms,output_tokens,updated_at)
         SELECT t.source_id,'turn:'||t.turn_id,CASE WHEN lower(t.model) LIKE 'gpt-%' THEN 'OpenAI' ELSE 'unknown' END,
                t.model,COALESCE(NULLIF(lower(t.reasoning_effort),''),'default'),
                CAST(strftime('%s',t.completed_at) AS INTEGER)*1000,t.duration_ms,t.ttft_ms,t.output_tokens,t.updated_at
         FROM turns t JOIN sources s ON s.id=t.source_id
         WHERE s.source_kind='codex_jsonl' AND t.status='completed' AND t.completed_at IS NOT NULL AND t.duration_ms IS NOT NULL",
        [],
    ).map_err(db::to_error)?;
    Ok(())
}

pub fn sync_codex_incremental(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(db::to_error)?;
    let result = sync_codex_turns(conn)
        .and_then(|_| materialize_observations(conn))
        .and_then(|_| reprice_conn(conn));
    if result.is_ok() {
        conn.execute_batch("COMMIT").map_err(db::to_error)?;
    } else {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
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

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DshUsage {
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DshMessage {
    id: Option<String>,
    content: Option<IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DshChunk {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<IgnoredAny>,
    block: Option<IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DshData {
    turn: Option<i64>,
    step: Option<i64>,
    model: Option<String>,
    provider: Option<String>,
    reasoning_effort: Option<String>,
    usage: Option<DshUsage>,
    message: Option<DshMessage>,
    chunk: Option<DshChunk>,
    header: Option<IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DshEvent {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    time: Option<i64>,
    data: Option<DshData>,
}

#[derive(Default)]
struct DshStep {
    turn: i64,
    step: i64,
    started: i64,
    first: Option<i64>,
    provider: String,
    model: String,
    effort: String,
    message_id: Option<String>,
    usage: Option<DshUsage>,
}

fn normalized_dsh_input(usage: &DshUsage) -> i64 {
    let separated = usage
        .input_tokens
        .saturating_add(usage.cache_read_tokens)
        .saturating_add(usage.cache_write_tokens)
        .saturating_add(usage.output_tokens);
    if usage.total_tokens >= separated {
        usage.input_tokens.max(0)
    } else {
        (usage.input_tokens - usage.cache_read_tokens - usage.cache_write_tokens).max(0)
    }
}

fn file_stamp(conn: &Connection, source_id: i64, path: &Path) -> AppResult<(bool, u64, i64)> {
    let metadata = path.metadata().map_err(db::to_error)?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0);
    let unchanged = conn
        .query_row(
            "SELECT size=?2 AND mtime_ms=?3 FROM scan_files WHERE source_id=?1 AND path=?4",
            params![source_id, size as i64, mtime, path.to_string_lossy()],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(db::to_error)?
        .unwrap_or(false);
    Ok((unchanged, size, mtime))
}

fn mark_file(
    conn: &Connection,
    source_id: i64,
    path: &Path,
    size: u64,
    mtime: i64,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO scan_files(source_id,path,size,mtime_ms,offset,last_scan_at)
         VALUES (?1,?2,?3,?4,?3,?5)
         ON CONFLICT(path) DO UPDATE SET source_id=excluded.source_id,size=excluded.size,
           mtime_ms=excluded.mtime_ms,offset=excluded.offset,last_scan_at=excluded.last_scan_at,error=NULL",
        params![
            source_id,
            path.to_string_lossy(),
            size as i64,
            mtime,
            Utc::now().to_rfc3339()
        ],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn sync_dsh(conn: &Connection) -> AppResult<()> {
    let Some((source_id, root)) = source(conn, "dsh_zstd")? else {
        return Ok(());
    };
    let root = Path::new(&root);
    if !root.is_dir() {
        return Ok(());
    }
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".jsonl.zstd"))
        })
    {
        let path = entry.path();
        let (unchanged, size, mtime) = file_stamp(conn, source_id, path)?;
        if unchanged {
            continue;
        }
        if import_dsh_file(conn, source_id, path)? {
            mark_file(conn, source_id, path, size, mtime)?;
        }
    }
    conn.execute(
        "UPDATE sources SET last_scan_at=?1,error=NULL WHERE id=?2",
        params![Utc::now().to_rfc3339(), source_id],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn import_dsh_file(conn: &Connection, source_id: i64, path: &Path) -> AppResult<bool> {
    let decoder = zstd::stream::read::Decoder::new(File::open(path).map_err(db::to_error)?)
        .map_err(db::to_error)?;
    let mut reader = BufReader::new(decoder);
    let mut buffer = Vec::with_capacity(16 * 1024);
    let mut session_id = path
        .parent()
        .and_then(|value| value.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut selected = (
        "unknown".to_string(),
        "unknown".to_string(),
        "default".to_string(),
    );
    let mut steps: HashMap<(i64, i64), DshStep> = HashMap::new();
    let mut complete = true;
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                complete = false;
                break;
            }
        }
        let Ok(event) = serde_json::from_slice::<DshEvent>(&buffer) else {
            continue;
        };
        if event.kind == "session" {
            if let Some(id) = event.id {
                session_id = id;
            }
            continue;
        }
        let Some(data) = event.data else {
            continue;
        };
        if matches!(event.kind.as_str(), "model/selection" | "request/context") {
            if let Some(provider) = data.provider {
                selected.0 = provider;
            }
            if let Some(model) = data.model {
                selected.1 = model;
            }
            if let Some(effort) = data.reasoning_effort {
                selected.2 = normalize_effort(Some(&effort));
            }
            continue;
        }
        let (Some(turn), Some(step)) = (data.turn, data.step) else {
            continue;
        };
        let key = (turn, step);
        match event.kind.as_str() {
            "step/start" => {
                steps.insert(
                    key,
                    DshStep {
                        turn,
                        step,
                        started: event.time.unwrap_or(0),
                        provider: selected.0.clone(),
                        model: selected.1.clone(),
                        effort: selected.2.clone(),
                        ..DshStep::default()
                    },
                );
            }
            "assistant/chunk" => {
                if let Some(state) = steps.get_mut(&key) {
                    let meaningful = data
                        .chunk
                        .as_ref()
                        .and_then(|chunk| chunk.kind.as_deref())
                        .is_some_and(|kind| !matches!(kind, "usage" | "finish"));
                    if meaningful && state.first.is_none() {
                        state.first = event.time;
                    }
                }
            }
            "assistant/message" => {
                if let Some(state) = steps.get_mut(&key) {
                    state.message_id = data.message.and_then(|message| message.id);
                    state.usage = data.usage;
                    if state.first.is_none() {
                        state.first = event.time;
                    }
                }
            }
            "step/end" => {
                if let Some(mut state) = steps.remove(&key) {
                    if let Some(usage) = state.usage.take() {
                        let completed = event.time.unwrap_or(state.started);
                        let external_id = state.message_id.unwrap_or_else(|| {
                            format!("{session_id}:{}:{}", state.turn, state.step)
                        });
                        upsert_observation(
                            conn,
                            source_id,
                            &external_id,
                            &state.provider,
                            &state.model,
                            &state.effort,
                            state.started,
                            Some(completed),
                            Some((completed - state.started).max(0)),
                            state.first.map(|first| (first - state.started).max(0)),
                            "completed",
                            normalized_dsh_input(&usage),
                            usage.cache_read_tokens.max(0),
                            usage.cache_write_tokens.max(0),
                            usage.output_tokens.max(0),
                            usage.reasoning_tokens.max(0),
                            &Utc.timestamp_millis_opt(completed)
                                .single()
                                .unwrap_or_else(Utc::now)
                                .to_rfc3339(),
                        )?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(complete)
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EvoUsage {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EvoModel {
    id: String,
    provider: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EvoModelValue {
    Object(EvoModel),
    Name(String),
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EvoDetails {
    event_name: Option<String>,
    event: Option<String>,
    model: Option<EvoModelValue>,
    provider: Option<String>,
    model_call_id: Option<String>,
    provider_response_id: Option<String>,
    response_id: Option<String>,
    input_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    usage: Option<EvoUsage>,
    messages: Option<IgnoredAny>,
    tools: Option<IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EvoEvent {
    kind: String,
    ts_iso: String,
    session_id: Option<String>,
    task_id: Option<String>,
    step_seq: Option<i64>,
    details: EvoDetails,
}

#[derive(Default, Clone)]
struct EvoResponse {
    started: i64,
    completed: i64,
    provider: String,
    model: String,
    response_id: String,
    usage: (i64, i64, i64, i64),
}

fn evo_model(value: Option<&EvoModelValue>, provider: Option<&str>) -> (String, String) {
    match value {
        Some(EvoModelValue::Object(model)) => (model.provider.clone(), model.id.clone()),
        Some(EvoModelValue::Name(model)) => {
            (provider.unwrap_or("unknown").to_string(), model.clone())
        }
        None => (provider.unwrap_or("unknown").to_string(), "unknown".into()),
    }
}

fn sync_evox(conn: &Connection) -> AppResult<()> {
    let Some((source_id, root)) = source(conn, "evox_observability")? else {
        return Ok(());
    };
    let root = Path::new(&root);
    if !root.is_dir() {
        return Ok(());
    }
    for entry in WalkDir::new(root)
        .max_depth(1)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
        })
    {
        let path = entry.path();
        let (unchanged, size, mtime) = file_stamp(conn, source_id, path)?;
        if unchanged {
            continue;
        }
        import_evox_file(conn, source_id, path)?;
        mark_file(conn, source_id, path, size, mtime)?;
    }
    conn.execute(
        "UPDATE sources SET last_scan_at=?1,error=NULL WHERE id=?2",
        params![Utc::now().to_rfc3339(), source_id],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn import_evox_file(conn: &Connection, source_id: i64, path: &Path) -> AppResult<()> {
    let mut reader = BufReader::new(File::open(path).map_err(db::to_error)?);
    let mut buffer = Vec::with_capacity(16 * 1024);
    let mut requests: HashMap<(String, String), (i64, String, String)> = HashMap::new();
    let mut responses: HashMap<String, EvoResponse> = HashMap::new();
    loop {
        buffer.clear();
        if reader
            .read_until(b'\n', &mut buffer)
            .map_err(db::to_error)?
            == 0
        {
            break;
        }
        let Ok(event) = serde_json::from_slice::<EvoEvent>(&buffer) else {
            continue;
        };
        let event_name = event
            .details
            .event_name
            .as_deref()
            .or(event.details.event.as_deref())
            .unwrap_or("");
        let session = event.session_id.unwrap_or_default();
        let task = event
            .task_id
            .unwrap_or_else(|| event.step_seq.unwrap_or(0).to_string());
        let timestamp = parse_iso_ms(&event.ts_iso).unwrap_or(0);
        if event.kind == "llm_request" && event_name == "request_dispatch" {
            let (provider, model) = evo_model(
                event.details.model.as_ref(),
                event.details.provider.as_deref(),
            );
            requests.insert((session, task), (timestamp, provider, model));
        } else if event.kind == "llm_response" && event_name == "response_complete" {
            let response_id = event
                .details
                .response_id
                .clone()
                .or(event.details.provider_response_id.clone())
                .unwrap_or_else(|| format!("{session}:{task}:{}", event.step_seq.unwrap_or(0)));
            let (provider, model) = evo_model(
                event.details.model.as_ref(),
                event.details.provider.as_deref(),
            );
            let (started, request_provider, request_model) = requests
                .get(&(session.clone(), task.clone()))
                .cloned()
                .unwrap_or((timestamp, provider.clone(), model.clone()));
            let usage = event
                .details
                .usage
                .map(|usage| {
                    (
                        usage.input,
                        usage.cache_read,
                        usage.cache_write,
                        usage.output,
                    )
                })
                .unwrap_or_default();
            responses.insert(
                response_id.clone(),
                EvoResponse {
                    started,
                    completed: timestamp,
                    provider: if provider == "unknown" {
                        request_provider
                    } else {
                        provider
                    },
                    model: if model == "unknown" {
                        request_model
                    } else {
                        model
                    },
                    response_id,
                    usage,
                },
            );
        } else if event.kind == "llm_response" && event_name == "token_usage_recorded" {
            let response_id = event
                .details
                .provider_response_id
                .clone()
                .or(event.details.response_id.clone())
                .unwrap_or_default();
            let response = responses.remove(&response_id);
            let (provider, model) = evo_model(
                event.details.model.as_ref(),
                event.details.provider.as_deref(),
            );
            let started = response
                .as_ref()
                .map(|value| value.started)
                .unwrap_or(timestamp);
            let completed = response
                .as_ref()
                .map(|value| value.completed)
                .unwrap_or(timestamp);
            let external_id = event.details.model_call_id.clone().unwrap_or_else(|| {
                if response_id.is_empty() {
                    format!("{session}:{task}:{}", event.step_seq.unwrap_or(0))
                } else {
                    response_id.clone()
                }
            });
            let input = event.details.input_tokens.unwrap_or(0).max(0);
            let cache = event.details.cached_input_tokens.unwrap_or(0).max(0);
            let output = event.details.output_tokens.unwrap_or(0).max(0);
            let resolved_provider = if provider == "unknown" {
                response
                    .as_ref()
                    .map(|value| value.provider.as_str())
                    .unwrap_or("unknown")
            } else {
                &provider
            };
            let resolved_model = if model == "unknown" {
                response
                    .as_ref()
                    .map(|value| value.model.as_str())
                    .unwrap_or("unknown")
            } else {
                &model
            };
            upsert_observation(
                conn,
                source_id,
                &external_id,
                resolved_provider,
                resolved_model,
                "default",
                started,
                Some(completed),
                Some((completed - started).max(0)),
                None,
                "completed",
                input,
                cache,
                0,
                output,
                0,
                &event.ts_iso,
            )?;
        }
    }
    for (_, response) in responses {
        let (input, cache_read, cache_write, output) = response.usage;
        upsert_observation(
            conn,
            source_id,
            &response.response_id,
            &response.provider,
            &response.model,
            "default",
            response.started,
            Some(response.completed),
            Some((response.completed - response.started).max(0)),
            None,
            "completed",
            input.max(0),
            cache_read.max(0),
            cache_write.max(0),
            output.max(0),
            0,
            &Utc.timestamp_millis_opt(response.completed)
                .single()
                .unwrap_or_else(Utc::now)
                .to_rfc3339(),
        )?;
    }
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

#[cfg(test)]
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

#[derive(Clone)]
struct DbRate {
    id: i64,
    model: String,
    aliases: Vec<String>,
    input: i64,
    cached_read: Option<i64>,
    cached_write: Option<i64>,
    output: i64,
    effective_from: String,
    effective_to: Option<String>,
}

fn load_db_rates(conn: &Connection) -> AppResult<Vec<DbRate>> {
    let mut stmt = conn.prepare(
        "SELECT id,vendor,model,input_pico_per_token,cached_read_pico_per_token,cached_write_pico_per_token,output_pico_per_token,effective_from,effective_to
         FROM pricing_rates WHERE deleted_at IS NULL ORDER BY effective_from",
    ).map_err(db::to_error)?;
    let base = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(db::to_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db::to_error)?;
    let mut result = Vec::with_capacity(base.len());
    for (
        id,
        vendor,
        model,
        input,
        cached_read,
        cached_write,
        output,
        effective_from,
        effective_to,
    ) in base
    {
        let mut alias_stmt = conn
            .prepare("SELECT alias FROM pricing_rate_aliases WHERE rate_id=?1 ORDER BY alias")
            .map_err(db::to_error)?;
        let aliases = alias_stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(db::to_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db::to_error)?;
        let _ = vendor;
        result.push(DbRate {
            id,
            model,
            aliases,
            input,
            cached_read,
            cached_write,
            output,
            effective_from,
            effective_to,
        });
    }
    Ok(result)
}

fn db_rate_for<'a>(rates: &'a [DbRate], model: &str, date: &str) -> Option<&'a DbRate> {
    rates
        .iter()
        .filter(|rate| rate.effective_from.as_str() <= date)
        .filter(|rate| rate.effective_to.as_deref().is_none_or(|end| date < end))
        .filter(|rate| {
            rate.model.eq_ignore_ascii_case(model)
                || rate
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(model))
        })
        .max_by_key(|rate| rate.effective_from.as_str())
}

fn reprice_conn(conn: &Connection) -> AppResult<()> {
    let rates = load_db_rates(conn)?;
    let mut stmt = conn.prepare("SELECT source_id,external_id,provider,model,occurred_at_ms,uncached_input_tokens,cached_read_tokens,cached_write_tokens,output_tokens,total_tokens FROM model_call_observations").map_err(db::to_error)?;
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
            (0, total, "free", None::<i64>)
        } else if let Some(rate) = db_rate_for(&rates, &model, &date) {
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
            (cost, priced, status, Some(rate.id))
        } else {
            (0, 0, "unpriced", None)
        };
        conn.execute("UPDATE model_call_observations SET estimated_cost_nano_usd=?1,priced_tokens=?2,pricing_status=?3,pricing_rate_id=?4 WHERE source_id=?5 AND external_id=?6", params![cost,priced,status,key,source_id,id]).map_err(db::to_error)?;
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
    let (priced,total) = conn.query_row("SELECT count(*) FILTER (WHERE pricing_status IN ('priced','free')),count(*) FROM model_call_observations", [], |row| Ok((row.get(0)?,row.get(1)?))).map_err(db::to_error)?;
    let rates = pricing_rates_conn(conn)?;
    Ok(PricingCatalogStatus {
        version: CATALOG_VERSION.into(),
        currency: "USD".into(),
        verified_at: VERIFIED_AT.into(),
        rates,
        priced_observations: priced,
        total_observations: total,
    })
}

fn pricing_rates_conn(conn: &Connection) -> AppResult<Vec<PricingRate>> {
    let mut stmt = conn.prepare(
        "SELECT r.id,r.vendor,r.model,r.input_pico_per_token,r.cached_read_pico_per_token,r.cached_write_pico_per_token,r.output_pico_per_token,r.effective_from,r.effective_to,r.source_url,r.verified_at,r.origin,
                (SELECT count(*) FROM model_call_observations o WHERE o.pricing_rate_id=r.id)
         FROM pricing_rates r WHERE r.deleted_at IS NULL ORDER BY lower(r.model),r.effective_from DESC",
    ).map_err(db::to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, i64>(12)?,
            ))
        })
        .map_err(db::to_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db::to_error)?;
    let mut result = Vec::with_capacity(rows.len());
    for (
        id,
        vendor,
        model,
        input,
        cached_read,
        cached_write,
        output,
        effective_from,
        effective_to,
        source_url,
        verified_at,
        origin,
        call_count,
    ) in rows
    {
        let mut alias_stmt = conn
            .prepare("SELECT alias FROM pricing_rate_aliases WHERE rate_id=?1 ORDER BY alias")
            .map_err(db::to_error)?;
        let aliases = alias_stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(db::to_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db::to_error)?;
        result.push(PricingRate {
            id,
            vendor,
            model,
            aliases,
            currency: "USD".into(),
            input_usd_per_million: rate_string(input),
            cached_read_usd_per_million: cached_read.map(rate_string),
            cached_write_usd_per_million: cached_write.map(rate_string),
            output_usd_per_million: rate_string(output),
            effective_from,
            effective_to,
            source_url,
            verified_at,
            origin,
            call_count,
        });
    }
    Ok(result)
}

fn parse_price(value: &str, field: &str) -> AppResult<i64> {
    let value = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{field} 必须是非负数字"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{field} 必须是非负数字"));
    }
    Ok((value * 1_000_000.0).round() as i64)
}

fn parse_optional_price(value: Option<&str>, field: &str) -> AppResult<Option<i64>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| parse_price(value, field))
        .transpose()
}

fn validate_rate(
    conn: &Connection,
    input: &PricingRateInput,
    exclude_id: Option<i64>,
) -> AppResult<()> {
    if input.vendor.trim().is_empty() || input.model.trim().is_empty() {
        return Err("Vendor 和模型不能为空".into());
    }
    NaiveDate::parse_from_str(&input.effective_from, "%Y-%m-%d")
        .map_err(|_| "生效日期格式必须为 YYYY-MM-DD".to_string())?;
    if let Some(end) = input
        .effective_to
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .map_err(|_| "结束日期格式必须为 YYYY-MM-DD".to_string())?;
        if end <= input.effective_from.as_str() {
            return Err("结束日期必须晚于生效日期".into());
        }
    }
    let names = std::iter::once(input.model.trim())
        .chain(input.aliases.iter().map(String::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    let rates = load_db_rates(conn)?;
    for rate in rates.into_iter().filter(|rate| Some(rate.id) != exclude_id) {
        let rate_names =
            std::iter::once(rate.model.as_str()).chain(rate.aliases.iter().map(String::as_str));
        if !names.iter().any(|name| {
            rate_names
                .clone()
                .any(|other| name.eq_ignore_ascii_case(other))
        }) {
            continue;
        }
        let new_end = input
            .effective_to
            .as_deref()
            .filter(|v| !v.is_empty())
            .unwrap_or("9999-12-31");
        let old_end = rate.effective_to.as_deref().unwrap_or("9999-12-31");
        if input.effective_from.as_str() < old_end && rate.effective_from.as_str() < new_end {
            return Err(format!("{} 的价格生效区间与现有版本重叠", input.model));
        }
    }
    Ok(())
}

fn write_aliases(conn: &Connection, id: i64, aliases: &[String]) -> AppResult<()> {
    conn.execute("DELETE FROM pricing_rate_aliases WHERE rate_id=?1", [id])
        .map_err(db::to_error)?;
    for alias in aliases.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        conn.execute(
            "INSERT OR IGNORE INTO pricing_rate_aliases(rate_id,alias) VALUES (?1,?2)",
            params![id, alias],
        )
        .map_err(db::to_error)?;
    }
    Ok(())
}

pub fn create_pricing_rate(path: &Path, input: PricingRateInput) -> AppResult<PricingRate> {
    let conn = db::open(path)?;
    validate_rate(&conn, &input, None)?;
    let now = Utc::now().to_rfc3339();
    conn.execute("INSERT INTO pricing_rates(vendor,model,input_pico_per_token,cached_read_pico_per_token,cached_write_pico_per_token,output_pico_per_token,effective_from,effective_to,source_url,verified_at,origin,user_modified,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'custom',1,?10,?10)", params![input.vendor.trim(),input.model.trim(),parse_price(&input.input_usd_per_million,"输入价格")?,parse_optional_price(input.cached_read_usd_per_million.as_deref(),"缓存读取价格")?,parse_optional_price(input.cached_write_usd_per_million.as_deref(),"缓存写入价格")?,parse_price(&input.output_usd_per_million,"输出价格")?,input.effective_from,input.effective_to.as_deref().filter(|v| !v.is_empty()),input.source_url.trim(),now]).map_err(db::to_error)?;
    let id = conn.last_insert_rowid();
    write_aliases(&conn, id, &input.aliases)?;
    reprice_conn(&conn)?;
    pricing_rates_conn(&conn)?
        .into_iter()
        .find(|rate| rate.id == id)
        .ok_or_else(|| "价格保存后未找到".into())
}

pub fn update_pricing_rate(
    path: &Path,
    id: i64,
    input: PricingRateInput,
) -> AppResult<PricingRate> {
    let conn = db::open(path)?;
    validate_rate(&conn, &input, Some(id))?;
    let now = Utc::now().to_rfc3339();
    let changed=conn.execute("UPDATE pricing_rates SET vendor=?1,model=?2,input_pico_per_token=?3,cached_read_pico_per_token=?4,cached_write_pico_per_token=?5,output_pico_per_token=?6,effective_from=?7,effective_to=?8,source_url=?9,verified_at=?10,origin='custom',user_modified=1,updated_at=?10 WHERE id=?11 AND deleted_at IS NULL",params![input.vendor.trim(),input.model.trim(),parse_price(&input.input_usd_per_million,"输入价格")?,parse_optional_price(input.cached_read_usd_per_million.as_deref(),"缓存读取价格")?,parse_optional_price(input.cached_write_usd_per_million.as_deref(),"缓存写入价格")?,parse_price(&input.output_usd_per_million,"输出价格")?,input.effective_from,input.effective_to.as_deref().filter(|v| !v.is_empty()),input.source_url.trim(),now,id]).map_err(db::to_error)?;
    if changed == 0 {
        return Err("价格版本不存在".into());
    }
    write_aliases(&conn, id, &input.aliases)?;
    reprice_conn(&conn)?;
    pricing_rates_conn(&conn)?
        .into_iter()
        .find(|rate| rate.id == id)
        .ok_or_else(|| "价格更新后未找到".into())
}

pub fn delete_pricing_rate(path: &Path, id: i64) -> AppResult<()> {
    let conn = db::open(path)?;
    let row=conn.query_row("SELECT model,effective_from,origin FROM pricing_rates WHERE id=?1 AND deleted_at IS NULL",[id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).optional().map_err(db::to_error)?.ok_or_else(||"价格版本不存在".to_string())?;
    let now = Utc::now().to_rfc3339();
    if row.2 == "builtin" {
        conn.execute("INSERT OR REPLACE INTO pricing_seed_tombstones(model,effective_from,deleted_at) VALUES (?1,?2,?3)",params![row.0,row.1,now]).map_err(db::to_error)?;
    }
    conn.execute(
        "UPDATE pricing_rates SET deleted_at=?1,updated_at=?1 WHERE id=?2",
        params![now, id],
    )
    .map_err(db::to_error)?;
    reprice_conn(&conn)
}

struct PricingModelBuilder {
    model: String,
    vendor: String,
    call_count: i64,
    total_tokens: i64,
    pricing_status: String,
    rates: Vec<PricingRate>,
}

pub fn list_pricing_models(path: &Path) -> AppResult<Vec<PricingModel>> {
    let conn = db::open(path)?;
    let rates = pricing_rates_conn(&conn)?;
    let mut models: BTreeMap<String, PricingModelBuilder> = BTreeMap::new();
    let mut stmt=conn.prepare("SELECT model,provider,count(*),sum(total_tokens),CASE WHEN sum(priced_tokens)>=sum(total_tokens) THEN 'priced' WHEN sum(priced_tokens)>0 THEN 'partial' ELSE 'unpriced' END FROM model_call_observations GROUP BY model,provider").map_err(db::to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(db::to_error)?;
    for row in rows {
        let (model, vendor, calls, tokens, status) = row.map_err(db::to_error)?;
        models.insert(
            model.to_ascii_lowercase(),
            PricingModelBuilder {
                model,
                vendor,
                call_count: calls,
                total_tokens: tokens,
                pricing_status: status,
                rates: Vec::new(),
            },
        );
    }
    for rate in rates {
        let matching_keys = models
            .iter()
            .filter(|(_, item)| {
                rate.model.eq_ignore_ascii_case(&item.model)
                    || rate
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(&item.model))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if matching_keys.is_empty() {
            models.insert(
                rate.model.to_ascii_lowercase(),
                PricingModelBuilder {
                    model: rate.model.clone(),
                    vendor: rate.vendor.clone(),
                    call_count: 0,
                    total_tokens: 0,
                    pricing_status: "unused".into(),
                    rates: vec![rate],
                },
            );
        } else {
            for key in matching_keys {
                if let Some(entry) = models.get_mut(&key) {
                    entry.vendor = rate.vendor.clone();
                    entry.rates.push(rate.clone());
                }
            }
        }
    }
    Ok(models
        .into_values()
        .map(|item| PricingModel {
            model: item.model,
            vendor: item.vendor,
            call_count: item.call_count,
            total_tokens: item.total_tokens,
            pricing_status: item.pricing_status,
            rates: item.rates,
        })
        .collect())
}

pub fn get_data_integrity_status(path: &Path) -> AppResult<DataIntegrityStatus> {
    let conn = db::open(path)?;
    let mut stmt=conn.prepare("SELECT id,name,source_kind,error FROM sources WHERE enabled=1 AND name NOT IN ('Yodex','Lodex') ORDER BY id").map_err(db::to_error)?;
    let source_rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(db::to_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db::to_error)?;
    let mut sources = Vec::new();
    for (source_id, source_name, kind, source_error) in source_rows {
        let raw_call_count = if kind == "codex_jsonl" {
            conn.query_row("SELECT count(*) FROM model_calls c JOIN turns t ON t.turn_id=c.turn_id WHERE t.source_id=?1 AND c.total_tokens>0 AND (c.call_kind='primary' OR (c.call_kind='legacy' AND NOT EXISTS(SELECT 1 FROM model_calls p WHERE p.turn_id=c.turn_id AND p.call_kind='primary')))",[source_id],|row|row.get::<_,i64>(0)).map_err(db::to_error)?
        } else {
            conn.query_row("SELECT count(*) FROM usage_observations WHERE source_id=?1 AND status='completed' AND total_tokens>0",[source_id],|row|row.get::<_,i64>(0)).map_err(db::to_error)?
        };
        let indexed_call_count = conn
            .query_row(
                "SELECT count(*) FROM model_call_observations WHERE source_id=?1",
                [source_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db::to_error)?;
        let (unread_bytes,parse_error_count,file_error):(i64,i64,Option<String>)=conn.query_row("SELECT COALESCE(sum(max(size-offset,0)),0),COALESCE(sum(parse_error_count),0),max(error) FROM scan_files WHERE source_id=?1",[source_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(db::to_error)?;
        let last_ms = conn
            .query_row(
                "SELECT max(occurred_at_ms) FROM model_call_observations WHERE source_id=?1",
                [source_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(db::to_error)?;
        let difference = raw_call_count - indexed_call_count;
        let error = source_error.or(file_error);
        let reconciled =
            difference == 0 && unread_bytes == 0 && parse_error_count == 0 && error.is_none();
        sources.push(SourceIntegrityStatus {
            source_id,
            source_name,
            raw_call_count,
            indexed_call_count,
            difference,
            unread_bytes,
            parse_error_count,
            last_call_at: last_ms
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
                .map(|v| v.to_rfc3339()),
            sync_delay_ms: Some(if unread_bytes == 0 {
                0
            } else {
                last_ms
                    .map(|ms| (Utc::now().timestamp_millis() - ms).max(0))
                    .unwrap_or(0)
            }),
            reconciled,
            error,
        });
    }
    Ok(DataIntegrityStatus {
        reconciled: sources.iter().all(|source| source.reconciled),
        sources,
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
    let mut sql = "SELECT o.source_id,s.name,o.provider,o.model,o.reasoning_effort,o.occurred_at_ms,o.uncached_input_tokens,o.cached_read_tokens,o.cached_write_tokens,o.output_tokens,o.reasoning_tokens,o.total_tokens,o.estimated_cost_nano_usd,o.priced_tokens,o.pricing_status,o.updated_at FROM model_call_observations o JOIN sources s ON s.id=o.source_id WHERE s.enabled=1 AND o.total_tokens>0".to_string();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(start) = period_start(filters.period) {
        sql.push_str(" AND o.occurred_at_ms>=?");
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
    sql.push_str(" ORDER BY o.occurred_at_ms DESC");
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
                duration_ms: None,
                ttft_ms: None,
                tokens: UsageTokens {
                    uncached_input: row.get(6)?,
                    cached_read: row.get(7)?,
                    cached_write: row.get(8)?,
                    output: row.get(9)?,
                    reasoning: row.get(10)?,
                    total: row.get(11)?,
                },
                cost: row.get(12)?,
                priced_tokens: row.get(13)?,
                pricing_status: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })
        .map_err(db::to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db::to_error)
}

fn load_performance(path: &Path, filters: &AnalyticsFilters) -> AppResult<Vec<Observation>> {
    let conn = db::open(path)?;
    let mut sql="SELECT o.source_id,s.name,o.provider,o.model,o.reasoning_effort,o.occurred_at_ms,o.duration_ms,o.ttft_ms,o.output_tokens,o.updated_at FROM performance_observations o JOIN sources s ON s.id=o.source_id WHERE s.enabled=1".to_string();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(start) = period_start(filters.period) {
        sql.push_str(" AND o.occurred_at_ms>=?");
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
    sql.push_str(" ORDER BY o.occurred_at_ms DESC");
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
                    output: row.get(8)?,
                    total: row.get(8)?,
                    ..UsageTokens::default()
                },
                cost: 0,
                priced_tokens: 0,
                pricing_status: "performance".into(),
                updated_at: row.get(9)?,
            })
        })
        .map_err(db::to_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db::to_error)
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// 有效解码窗口下限：小于该值的样本视为计时口径异常，不参与速度统计。
const MIN_DECODE_MS: i64 = 500;

fn decode_ms(row: &Observation) -> Option<i64> {
    let duration = row.duration_ms?;
    let ttft = row.ttft_ms?;
    (row.tokens.output > 0 && duration > ttft && duration - ttft >= MIN_DECODE_MS)
        .then_some(duration - ttft)
}

/// 按 Token 加权的有效速度：小分母异常样本不再以算术平均拉高整体。
fn weighted_tps(rows: &[Observation]) -> Option<f64> {
    let mut tokens = 0_i64;
    let mut weighted_sum = 0.0;
    for row in rows {
        if let Some(window) = decode_ms(row) {
            tokens += row.tokens.output;
            let output = row.tokens.output as f64;
            weighted_sum += output * (output / (window as f64 / 1000.0));
        }
    }
    (tokens > 0).then(|| weighted_sum / tokens as f64)
}

fn summarize(
    period: MetricPeriod,
    rows: &[Observation],
    performance: &[Observation],
) -> MetricSummary {
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
        call_count: rows.len() as i64,
        performance_sample_count: performance.len() as i64,
        average_ttft_ms: average(
            performance
                .iter()
                .filter_map(|r| r.ttft_ms.map(|v| v as f64)),
        ),
        average_effective_tps: weighted_tps(performance),
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
    let performance = load_performance(path, &filters)?;
    Ok(summarize(filters.period, &rows, &performance))
}

pub fn query_model_effort_stats(
    path: &Path,
    filters: AnalyticsFilters,
) -> AppResult<Vec<ModelEffortStat>> {
    let rows = load(path, &filters)?;
    let performance = load_performance(path, &filters)?;
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
                let perf = performance
                    .iter()
                    .filter(|row| {
                        row.source_id == source_id
                            && row.provider == provider
                            && row.model == model
                            && row.effort == effort
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let summary = summarize(filters.period, &rows, &perf);
                ModelEffortStat {
                    source_id,
                    source_name,
                    provider,
                    model,
                    reasoning_effort: effort,
                    call_count: summary.call_count,
                    performance_sample_count: summary.performance_sample_count,
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
            let performance = load_performance(path, &filters).unwrap_or_default();
            let perf = performance
                .into_iter()
                .filter(|row| {
                    let local = Local.timestamp_millis_opt(row.completed_at).single();
                    match (filters.period, local) {
                        (MetricPeriod::Realtime, _) => rows
                            .iter()
                            .any(|call| call.source_id == row.source_id && call.model == row.model),
                        (MetricPeriod::Today, Some(v)) => {
                            v.format("%Y-%m-%d-%H").to_string() == bucket
                        }
                        (MetricPeriod::Week | MetricPeriod::Month, Some(v)) => {
                            v.format("%Y-%m-%d").to_string() == bucket
                        }
                        (MetricPeriod::Year, Some(v)) => v.format("%Y-%m").to_string() == bucket,
                        _ => false,
                    }
                })
                .collect::<Vec<_>>();
            let s = summarize(filters.period, &rows, &perf);
            MetricSeriesPoint {
                bucket,
                label,
                call_count: s.call_count,
                performance_sample_count: s.performance_sample_count,
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
    fn spark_contributor_rates_match_official_announcement() {
        let rate = rate_for("muse-spark-1.3-contributor", "2026-09-03").expect("contributor rate");
        assert_eq!(rate.vendor, "Meta");
        assert_eq!(rate.model, "muse-spark-1.3");
        assert_eq!(rate.input, 100_000);
        assert_eq!(rate.output, 200_000);
        assert_eq!(rate.cached_read, Some(2_000));
        assert_eq!(rate.cached_write, None);
        assert!(rate_for("muse-spark-1.3-contributor", "2026-09-02").is_none());
    }

    #[test]
    fn spark_free_alias_prices_like_contributor() {
        let base = rate_for("muse-spark-1.3-contributor", "2026-09-05").expect("contributor rate");
        let free =
            rate_for("muse-spark-1.3-contributor-free", "2026-09-05").expect("free alias rate");
        assert_eq!(base.model, free.model);
        assert_eq!(base.input, free.input);
        assert_eq!(base.output, free.output);
        assert_eq!(base.cached_read, free.cached_read);
        assert_eq!(base.effective_from, free.effective_from);
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
        let summary = summarize(MetricPeriod::Today, &rows, &[]);
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
        materialize_observations(&conn).unwrap();
        reprice_conn(&conn).unwrap();
        drop(conn);
        let summary = query_metric_summary(
            &path,
            AnalyticsFilters {
                period: MetricPeriod::Realtime,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(summary.call_count, 10);
        assert_eq!(summary.performance_sample_count, 10);
        assert_eq!(summary.tokens.total, 200);
        assert_eq!(summary.average_effective_tps, Some(10.0));
    }

    fn observation(output: i64, duration_ms: i64, ttft_ms: i64) -> Observation {
        Observation {
            source_id: 1,
            source_name: "Fixture".into(),
            provider: "omlx".into(),
            model: "Qwen-fixture".into(),
            effort: "default".into(),
            completed_at: 1,
            duration_ms: Some(duration_ms),
            ttft_ms: Some(ttft_ms),
            tokens: UsageTokens {
                output,
                total: output,
                ..Default::default()
            },
            cost: 0,
            priced_tokens: 0,
            pricing_status: "unpriced".into(),
            updated_at: "2026-09-06T00:00:00Z".into(),
        }
    }

    #[test]
    fn tiny_decode_windows_are_excluded_from_tps() {
        let rows = vec![
            observation(4_120, 22_816, 22_806),
            observation(1_000, 3_000, 1_000),
        ];
        let summary = summarize(MetricPeriod::Today, &[], &rows);
        assert_eq!(summary.average_effective_tps, Some(500.0));
    }

    #[test]
    fn average_tps_is_token_weighted_not_arithmetic() {
        let rows = vec![
            observation(100, 11_000, 1_000),
            observation(900, 31_000, 1_000),
        ];
        let summary = summarize(MetricPeriod::Today, &[], &rows);
        assert_eq!(summary.average_effective_tps, Some(28.0));
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

    #[test]
    fn dsh_zstd_imports_completed_step_without_duplicate_tokens() {
        use std::io::Write;

        let (temp, path) = empty_meter();
        let fixture = temp.path().join("session.jsonl.zstd");
        let lines = [
            r#"{"id":"session-1","type":"session","time":1000}"#,
            r#"{"type":"model/selection","time":1000,"data":{"provider":"OpenAI","model":"gpt-5.6-sol","reasoningEffort":"high"}}"#,
            r#"{"type":"step/start","time":1100,"data":{"turn":1,"step":2}}"#,
            r#"{"type":"assistant/chunk","time":1400,"data":{"turn":1,"step":2,"chunk":{"type":"text","text":"discard me"}}}"#,
            r#"{"type":"assistant/message","time":1800,"data":{"turn":1,"step":2,"message":{"id":"message-1","content":"discard me"},"usage":{"inputTokens":100,"cacheReadTokens":30,"cacheWriteTokens":10,"outputTokens":20,"reasoningTokens":5,"totalTokens":160}}}"#,
            r#"{"type":"step/end","time":2100,"data":{"turn":1,"step":2}}"#,
        ].join("\n") + "\n";
        let mut encoder =
            zstd::stream::write::Encoder::new(File::create(&fixture).unwrap(), 1).unwrap();
        encoder.write_all(lines.as_bytes()).unwrap();
        encoder.finish().unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (200,'DSH','/fixture','dsh_zstd',1)", []).unwrap();
        assert!(import_dsh_file(&conn, 200, &fixture).unwrap());
        assert!(import_dsh_file(&conn, 200, &fixture).unwrap());
        let row: (i64, String, String, String, i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT count(*),provider,model,reasoning_effort,ttft_ms,duration_ms,uncached_input_tokens,cached_read_tokens,total_tokens FROM usage_observations WHERE source_id=200",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?)),
        ).unwrap();
        assert_eq!(
            row,
            (
                1,
                "OpenAI".into(),
                "gpt-5.6-sol".into(),
                "high".into(),
                300,
                1000,
                100,
                30,
                160
            )
        );
    }

    #[test]
    fn dsh_truncated_frame_keeps_complete_observation_for_retry() {
        use std::io::Write;

        let (temp, path) = empty_meter();
        let fixture = temp.path().join("partial.jsonl.zstd");
        let lines = [
            r#"{"id":"partial-session","type":"session","time":1000}"#,
            r#"{"type":"step/start","time":1100,"data":{"turn":1,"step":1}}"#,
            r#"{"type":"assistant/message","time":1300,"data":{"turn":1,"step":1,"message":{"id":"partial-message"},"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15}}}"#,
            r#"{"type":"step/end","time":1500,"data":{"turn":1,"step":1}}"#,
        ]
        .join("\n")
            + "\n";
        let mut encoder =
            zstd::stream::write::Encoder::new(File::create(&fixture).unwrap(), 1).unwrap();
        encoder.write_all(lines.as_bytes()).unwrap();
        encoder.finish().unwrap();
        let mut tail_encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        tail_encoder
            .write_all(br#"{"type":"assistant/chunk","time":1600,"data":{"turn":2"#)
            .unwrap();
        let mut partial_tail = tail_encoder.finish().unwrap();
        partial_tail.truncate(partial_tail.len() - 2);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture)
            .unwrap()
            .write_all(&partial_tail)
            .unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (202,'DSH partial','/fixture','dsh_zstd',1)", []).unwrap();
        assert!(!import_dsh_file(&conn, 202, &fixture).unwrap());
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM usage_observations WHERE source_id=202",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn evox_prefers_token_record_and_deduplicates_model_call() {
        use std::io::Write;

        let (temp, path) = empty_meter();
        let fixture = temp.path().join("observability.jsonl");
        let token = r#"{"kind":"llm_response","ts_iso":"2026-09-06T00:00:03Z","session_id":"session-1","task_id":"task-1","step_seq":1,"details":{"event_name":"token_usage_recorded","model_call_id":"call-1","provider_response_id":"response-1","provider":"OpenAI","model":"gpt-5.6-sol","input_tokens":100,"cached_input_tokens":30,"output_tokens":20}}"#;
        let lines = [
            r#"{"kind":"llm_request","ts_iso":"2026-09-06T00:00:01Z","session_id":"session-1","task_id":"task-1","step_seq":1,"details":{"event_name":"request_dispatch","model":{"id":"gpt-5.6-sol","provider":"OpenAI"},"messages":["discard me"]}}"#,
            r#"{"kind":"llm_response","ts_iso":"2026-09-06T00:00:02Z","session_id":"session-1","task_id":"task-1","step_seq":1,"details":{"event_name":"response_complete","response_id":"response-1","model":{"id":"gpt-5.6-sol","provider":"OpenAI"},"usage":{"input":999,"output":999}}}"#,
            token,
            token,
        ].join("\n") + "\n";
        File::create(&fixture)
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (201,'EvoX','/fixture','evox_observability',1)", []).unwrap();
        import_evox_file(&conn, 201, &fixture).unwrap();
        let row: (i64, String, i64, i64, i64, Option<i64>) = conn.query_row(
            "SELECT count(*),external_id,uncached_input_tokens,cached_read_tokens,total_tokens,ttft_ms FROM usage_observations WHERE source_id=201",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        ).unwrap();
        assert_eq!(row, (1, "call-1".into(), 100, 30, 150, None));
    }

    #[test]
    fn codex_calls_use_response_ids_before_parent_turn_completes() {
        let (_temp, path) = empty_meter();
        let conn = db::open(&path).unwrap();
        conn.execute("INSERT INTO sources(id,name,root_path,source_kind,enabled) VALUES (300,'Codex','/fixture','codex_jsonl',1)",[]).unwrap();
        for (turn, status, completed) in [
            ("turn-primary", "running", None),
            ("turn-legacy", "completed", Some("2026-09-07T08:00:04Z")),
        ] {
            conn.execute("INSERT INTO turns(turn_id,session_id,source_id,started_at,started_local_date,completed_at,duration_ms,ttft_ms,model,reasoning_effort,status,updated_at) VALUES (?1,'session',300,'2026-09-07T08:00:00Z','2026-09-07',?2,4000,1000,'gpt-6-astra','medium',?3,'2026-09-07T08:00:04Z')",params![turn,completed,status]).unwrap();
        }
        let calls = [
            ("legacy-shadow", "turn-primary", "legacy", 80, 20, 10, 110),
            ("response-1", "turn-primary", "primary", 80, 20, 20, 120),
            ("response-2", "turn-primary", "primary", 150, 50, 30, 230),
            ("zero-heartbeat", "turn-primary", "primary", 0, 0, 0, 0),
            ("legacy-only", "turn-legacy", "legacy", 40, 10, 10, 60),
        ];
        for (id, turn, kind, input, cache, output, total) in calls {
            conn.execute("INSERT INTO model_calls(response_id,turn_id,session_id,occurred_at,call_kind,input_tokens,cached_input_tokens,output_tokens,reasoning_tokens,total_tokens) VALUES (?1,?2,'session','2026-09-07T08:00:02Z',?3,?4,?5,?6,0,?7)",params![id,turn,kind,input,cache,output,total]).unwrap();
        }
        materialize_observations(&conn).unwrap();
        reprice_conn(&conn).unwrap();
        let (count,total,cost):(i64,i64,i64)=conn.query_row("SELECT count(*),sum(total_tokens),sum(estimated_cost_nano_usd) FROM model_call_observations WHERE source_id=300",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!((count, total), (3, 410));
        assert!(cost > 0);
        let performance: i64 = conn
            .query_row(
                "SELECT count(*) FROM performance_observations WHERE source_id=300",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(performance, 1);
    }

    #[test]
    fn pricing_crud_preserves_blank_cache_and_deleted_seed() {
        let (_temp, path) = empty_meter();
        let input = PricingRateInput {
            vendor: "Test".into(),
            model: "custom-model".into(),
            aliases: vec!["custom-alias".into()],
            input_usd_per_million: "1".into(),
            cached_read_usd_per_million: None,
            cached_write_usd_per_million: Some("0".into()),
            output_usd_per_million: "2".into(),
            effective_from: "2026-01-01".into(),
            effective_to: None,
            source_url: "https://example.com/price".into(),
        };
        let created = create_pricing_rate(&path, input.clone()).unwrap();
        assert_eq!(created.cached_read_usd_per_million, None);
        assert_eq!(created.cached_write_usd_per_million, Some("0".into()));
        let overlapping = create_pricing_rate(
            &path,
            PricingRateInput {
                effective_from: "2026-06-01".into(),
                ..input.clone()
            },
        );
        assert!(overlapping.is_err());
        let astra_id = pricing_catalog_status(&path)
            .unwrap()
            .rates
            .into_iter()
            .find(|rate| rate.model == "gpt-6-astra")
            .unwrap()
            .id;
        delete_pricing_rate(&path, astra_id).unwrap();
        let conn = db::open(&path).unwrap();
        seed_pricing_rates(&conn).unwrap();
        let restored:i64=conn.query_row("SELECT count(*) FROM pricing_rates WHERE model='gpt-6-astra' AND deleted_at IS NULL",[],|row|row.get(0)).unwrap();
        assert_eq!(restored, 0);
    }
}
