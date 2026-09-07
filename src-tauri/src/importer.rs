use crate::db::{self, AppResult};
use crate::models::{
    EventMessage, ImportStatus, LogRecord, RecordBody, SessionMeta, TokenUsage, TurnContext,
};
use chrono::{DateTime, Local, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct BackendState {
    pub db_path: PathBuf,
    pub status: Arc<Mutex<ImportStatus>>,
    paused: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl BackendState {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            status: Arc::new(Mutex::new(ImportStatus {
                message: "等待导入".into(),
                ..ImportStatus::default()
            })),
            paused: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn status(&self) -> ImportStatus {
        self.status.lock().clone()
    }

    pub fn pause(&self) -> ImportStatus {
        self.paused.store(true, Ordering::Release);
        let mut status = self.status.lock();
        status.paused = true;
        status.message = "正在暂停".into();
        status.clone()
    }
}

#[derive(Debug)]
struct FileEntry {
    source_id: i64,
    path: PathBuf,
    size: u64,
    mtime_ms: i64,
}

#[derive(Debug, Default)]
struct ScanCursor {
    offset: u64,
    session_id: Option<String>,
    turn_id: Option<String>,
    previous_total: TokenUsage,
}

pub fn start_background_import(app: AppHandle, state: BackendState) -> bool {
    if state
        .running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    state.paused.store(false, Ordering::Release);
    std::thread::spawn(move || {
        let result = run_import(&app, &state);
        state.running.store(false, Ordering::Release);
        let mut status = state.status.lock();
        status.running = false;
        if state.paused.load(Ordering::Acquire) {
            status.paused = true;
            status.message = "已暂停，可随时继续".into();
        } else if let Err(error) = result {
            status.message = format!("导入失败：{error}");
            let _ = app.emit("source-error", error);
        } else {
            status.paused = false;
            status.current_file = None;
            status.message = "索引已更新".into();
            let _ = app.emit("metrics-updated", db::database_last_updated(&state.db_path));
        }
        let snapshot = status.clone();
        drop(status);
        let _ = app.emit("import-progress", snapshot);
    });
    true
}

fn run_import(app: &AppHandle, state: &BackendState) -> AppResult<()> {
    let files = collect_files(&state.db_path)?;
    let conn = db::open(&state.db_path)?;
    let bytes_done = files
        .iter()
        .map(|file| existing_offset(&conn, file).unwrap_or(0).min(file.size))
        .sum();
    {
        let mut status = state.status.lock();
        *status = ImportStatus {
            running: true,
            paused: false,
            files_done: 0,
            files_total: files.len() as u64,
            bytes_done,
            bytes_total: files.iter().map(|file| file.size).sum(),
            current_file: None,
            started_at: Some(db::now()),
            message: "正在建立本地索引".into(),
        };
        let _ = app.emit("import-progress", status.clone());
    }

    let mut last_metrics_emit = Instant::now() - Duration::from_secs(1);
    for (index, file) in files.iter().enumerate() {
        if state.paused.load(Ordering::Acquire) {
            break;
        }
        let before = existing_offset(&conn, file).unwrap_or(0).min(file.size);
        {
            let mut status = state.status.lock();
            status.current_file = Some(file.path.to_string_lossy().into_owned());
            status.message = format!("正在索引 {}/{}", index + 1, files.len());
        }
        let mut accounted = before;
        match scan_file(&conn, file, state, |offset| {
            let mut status = state.status.lock();
            status.bytes_done = status
                .bytes_done
                .saturating_sub(accounted)
                .saturating_add(offset.min(file.size));
            accounted = offset.min(file.size);
            let _ = app.emit("import-progress", status.clone());
        }) {
            Ok(offset) => {
                if offset > before {
                    crate::analytics::sync_codex_incremental(&conn)?;
                    if last_metrics_emit.elapsed() >= Duration::from_millis(300) {
                        let _ =
                            app.emit("metrics-updated", db::database_last_updated(&state.db_path));
                        last_metrics_emit = Instant::now();
                    }
                }
                let mut status = state.status.lock();
                status.bytes_done = status
                    .bytes_done
                    .saturating_sub(accounted)
                    .saturating_add(offset.min(file.size));
                status.files_done = if state.paused.load(Ordering::Acquire) {
                    index as u64
                } else {
                    index as u64 + 1
                };
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                conn.execute(
                    "UPDATE scan_files SET error=?1, last_scan_at=?2 WHERE path=?3",
                    params![error, db::now(), file.path.to_string_lossy()],
                )
                .map_err(db::to_error)?;
                let _ = app.emit(
                    "source-error",
                    format!("{}: {}", file.path.display(), error),
                );
            }
        }
    }
    let now = db::now();
    conn.execute(
        "UPDATE sources SET last_scan_at=?1, error=NULL WHERE enabled=1",
        params![now],
    )
    .map_err(db::to_error)?;
    drop(conn);
    crate::analytics::sync_all_sources(&state.db_path)?;
    Ok(())
}

fn collect_files(db_path: &Path) -> AppResult<Vec<FileEntry>> {
    let mut result = Vec::new();
    for (source_id, root) in db::enabled_sources(db_path)? {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
            if !entry.file_type().is_file() || !db::is_rollout(entry.path()) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or(0);
            result.push(FileEntry {
                source_id,
                path: entry.path().to_path_buf(),
                size: metadata.len(),
                mtime_ms,
            });
        }
    }
    let conn = db::open(db_path)?;
    result.sort_by(|a, b| {
        let a_unread = existing_offset(&conn, a).unwrap_or(0) < a.size;
        let b_unread = existing_offset(&conn, b).unwrap_or(0) < b.size;
        b_unread
            .cmp(&a_unread)
            .then_with(|| b.mtime_ms.cmp(&a.mtime_ms))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(result)
}

fn existing_offset(conn: &Connection, file: &FileEntry) -> AppResult<u64> {
    conn.query_row(
        "SELECT offset FROM scan_files WHERE path=?1",
        params![file.path.to_string_lossy()],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.unwrap_or(0).max(0) as u64)
    .map_err(db::to_error)
}

fn scan_file(
    conn: &Connection,
    file: &FileEntry,
    state: &BackendState,
    mut progress: impl FnMut(u64),
) -> AppResult<u64> {
    let path_text = file.path.to_string_lossy().into_owned();
    conn.execute(
        "INSERT INTO scan_files(source_id, path, size, mtime_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET source_id=excluded.source_id",
        params![file.source_id, path_text, file.size as i64, file.mtime_ms],
    )
    .map_err(db::to_error)?;
    let mut cursor = load_cursor(conn, &path_text)?;
    if file.size < cursor.offset {
        reset_file_data(conn, &path_text)?;
        cursor = ScanCursor::default();
    }
    if cursor.offset == file.size {
        mark_stale_turns(conn, &path_text, file.mtime_ms)?;
        return Ok(cursor.offset);
    }

    let mut input = File::open(&file.path).map_err(db::to_error)?;
    input
        .seek(SeekFrom::Start(cursor.offset))
        .map_err(db::to_error)?;
    let mut reader = BufReader::with_capacity(64 * 1024, input);
    let mut records_since_progress = 0_u64;
    let mut last_committed_offset = cursor.offset;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(db::to_error)?;

    loop {
        if state.paused.load(Ordering::Acquire) {
            save_cursor(conn, &path_text, file, &cursor, None)?;
            conn.execute_batch("COMMIT").map_err(db::to_error)?;
            return Ok(cursor.offset);
        }
        let record_start = reader.stream_position().map_err(db::to_error)?;
        let prefix = reader.fill_buf().map_err(db::to_error)?;
        if prefix.is_empty() {
            break;
        }
        if !should_parse_record(prefix) {
            let mut line = JsonLineReader::new(&mut reader);
            line.drain().map_err(db::to_error)?;
            cursor.offset = reader.stream_position().map_err(db::to_error)?;
            last_committed_offset = cursor.offset;
            records_since_progress += 1;
            if records_since_progress >= 500 {
                save_cursor(conn, &path_text, file, &cursor, None)?;
                conn.execute_batch("COMMIT; BEGIN IMMEDIATE")
                    .map_err(db::to_error)?;
                progress(cursor.offset);
                records_since_progress = 0;
            }
            continue;
        }
        let selected = fast_selected_record(prefix);
        let (parsed, ended_by_newline) = {
            let mut line = JsonLineReader::new(&mut reader);
            let parsed = if let Some(record) = selected {
                line.drain().map_err(db::to_error)?;
                Ok(record)
            } else {
                LogRecord::deserialize(&mut serde_json::Deserializer::from_reader(&mut line))
            };
            line.drain().map_err(db::to_error)?;
            (parsed, line.ended_by_newline)
        };
        let record_end = reader.stream_position().map_err(db::to_error)?;

        match parsed {
            Ok(record) => {
                process_record(
                    conn,
                    file.source_id,
                    &path_text,
                    record_end,
                    record,
                    &mut cursor,
                )?;
                cursor.offset = record_end;
                last_committed_offset = record_end;
            }
            Err(error) if !ended_by_newline && error.is_eof() => {
                cursor.offset = record_start;
                break;
            }
            Err(_) => {
                // A malformed complete JSONL record is skipped without retaining its contents.
                conn.execute(
                    "UPDATE scan_files SET parse_error_count=parse_error_count+1 WHERE path=?1",
                    params![path_text],
                )
                .map_err(db::to_error)?;
                cursor.offset = record_end;
                last_committed_offset = record_end;
            }
        }
        records_since_progress += 1;
        if records_since_progress >= 500 {
            save_cursor(conn, &path_text, file, &cursor, None)?;
            conn.execute_batch("COMMIT; BEGIN IMMEDIATE")
                .map_err(db::to_error)?;
            progress(cursor.offset);
            records_since_progress = 0;
        }
    }
    cursor.offset = last_committed_offset;
    save_cursor(conn, &path_text, file, &cursor, None)?;
    conn.execute_batch("COMMIT").map_err(db::to_error)?;
    mark_stale_turns(conn, &path_text, file.mtime_ms)?;
    Ok(cursor.offset)
}

fn should_parse_record(prefix: &[u8]) -> bool {
    let types = json_type_values(prefix, 2);
    match types.first().map(Vec::as_slice) {
        Some(b"session_meta" | b"turn_context" | b"token_usage_record") => true,
        Some(b"event_msg") => match types.get(1).map(Vec::as_slice) {
            Some(b"task_started" | b"task_complete" | b"token_count") => true,
            Some(_) => false,
            None => true,
        },
        Some(_) => false,
        None => true,
    }
}

fn fast_selected_record(prefix: &[u8]) -> Option<LogRecord> {
    match json_type_values(prefix, 1).first().map(Vec::as_slice) {
        Some(b"session_meta") => {
            let metadata = prefix_before_key(prefix, b"base_instructions");
            Some(LogRecord {
                timestamp: json_string_field(metadata, b"timestamp", 0),
                ordinal: json_u64_field(metadata, b"ordinal"),
                body: RecordBody::SessionMeta(SessionMeta::selected(
                    json_string_field(metadata, b"session_id", 0),
                    json_string_field(metadata, b"id", 0),
                    json_string_field(metadata, b"parent_thread_id", 0),
                    json_string_field(metadata, b"thread_source", 0),
                    json_string_field(metadata, b"agent_path", 0),
                    json_string_field(metadata, b"cwd", 0),
                    json_string_field(metadata, b"timestamp", 1),
                )),
            })
        }
        Some(b"turn_context") => {
            let turn_id = json_string_field(prefix, b"turn_id", 0)?;
            let model = json_string_field(prefix, b"model", 0)?;
            Some(LogRecord {
                timestamp: json_string_field(prefix, b"timestamp", 0),
                ordinal: json_u64_field(prefix, b"ordinal"),
                body: RecordBody::TurnContext(TurnContext::selected(
                    Some(turn_id),
                    json_string_field(prefix, b"cwd", 0),
                    Some(model),
                    json_string_field(prefix, b"effort", 0),
                )),
            })
        }
        _ => None,
    }
}

fn prefix_before_key<'a>(input: &'a [u8], key: &[u8]) -> &'a [u8] {
    let mut needle = Vec::with_capacity(key.len() + 2);
    needle.push(b'"');
    needle.extend_from_slice(key);
    needle.push(b'"');
    input
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|end| &input[..end])
        .unwrap_or(input)
}

fn json_string_field(input: &[u8], key: &[u8], occurrence: usize) -> Option<String> {
    let mut needle = Vec::with_capacity(key.len() + 2);
    needle.push(b'"');
    needle.extend_from_slice(key);
    needle.push(b'"');
    let mut offset = 0;
    for _ in 0..=occurrence {
        let relative = input[offset..]
            .windows(needle.len())
            .position(|window| window == needle)?;
        offset += relative + needle.len();
    }
    let mut cursor = offset;
    while input.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if input.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    while input.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if input.get(cursor) != Some(&b'"') {
        return None;
    }
    let start = cursor;
    cursor += 1;
    let mut escaped = false;
    while cursor < input.len() {
        let byte = input[cursor];
        if byte == b'"' && !escaped {
            return serde_json::from_slice(&input[start..=cursor]).ok();
        }
        escaped = byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        cursor += 1;
    }
    None
}

fn json_u64_field(input: &[u8], key: &[u8]) -> Option<u64> {
    let mut needle = Vec::with_capacity(key.len() + 2);
    needle.push(b'"');
    needle.extend_from_slice(key);
    needle.push(b'"');
    let relative = input
        .windows(needle.len())
        .position(|window| window == needle)?;
    let mut cursor = relative + needle.len();
    while input.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if input.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    while input.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let start = cursor;
    while input.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    std::str::from_utf8(&input[start..cursor])
        .ok()?
        .parse()
        .ok()
}

fn json_type_values(input: &[u8], limit: usize) -> Vec<Vec<u8>> {
    let mut values = Vec::with_capacity(limit);
    let mut cursor = 0;
    while cursor + 6 <= input.len() && values.len() < limit {
        let Some(relative) = input[cursor..]
            .windows(6)
            .position(|window| window == b"\"type\"")
        else {
            break;
        };
        cursor += relative + 6;
        while cursor < input.len() && input[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if input.get(cursor) != Some(&b':') {
            continue;
        }
        cursor += 1;
        while cursor < input.len() && input[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if input.get(cursor) != Some(&b'"') {
            continue;
        }
        cursor += 1;
        let start = cursor;
        while cursor < input.len() && input[cursor] != b'"' {
            cursor += 1;
        }
        if cursor >= input.len() {
            break;
        }
        values.push(input[start..cursor].to_vec());
        cursor += 1;
    }
    values
}

fn load_cursor(conn: &Connection, path: &str) -> AppResult<ScanCursor> {
    conn.query_row(
        "SELECT offset, current_session_id, current_turn_id,
                last_input, last_cached_input, last_output, last_reasoning, last_total
         FROM scan_files WHERE path=?1",
        params![path],
        |row| {
            Ok(ScanCursor {
                offset: row.get::<_, i64>(0)?.max(0) as u64,
                session_id: row.get(1)?,
                turn_id: row.get(2)?,
                previous_total: TokenUsage {
                    input_tokens: row.get(3)?,
                    cached_input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    reasoning_output_tokens: row.get(6)?,
                    total_tokens: row.get(7)?,
                },
            })
        },
    )
    .map_err(db::to_error)
}

fn save_cursor(
    conn: &Connection,
    path: &str,
    file: &FileEntry,
    cursor: &ScanCursor,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE scan_files SET size=?1, mtime_ms=?2, offset=?3,
                current_session_id=?4, current_turn_id=?5,
                last_input=?6, last_cached_input=?7, last_output=?8,
                last_reasoning=?9, last_total=?10, last_scan_at=?11, error=?12
         WHERE path=?13",
        params![
            file.size as i64,
            file.mtime_ms,
            cursor.offset as i64,
            cursor.session_id,
            cursor.turn_id,
            cursor.previous_total.input_tokens,
            cursor.previous_total.cached_input_tokens,
            cursor.previous_total.output_tokens,
            cursor.previous_total.reasoning_output_tokens,
            cursor.previous_total.total_tokens,
            db::now(),
            error,
            path
        ],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn reset_file_data(conn: &Connection, path: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM model_calls WHERE session_id IN
             (SELECT session_id FROM sessions WHERE file_path=?1)",
        params![path],
    )
    .map_err(db::to_error)?;
    conn.execute(
        "DELETE FROM turns WHERE session_id IN
             (SELECT session_id FROM sessions WHERE file_path=?1)",
        params![path],
    )
    .map_err(db::to_error)?;
    conn.execute("DELETE FROM sessions WHERE file_path=?1", params![path])
        .map_err(db::to_error)?;
    conn.execute(
        "UPDATE scan_files SET offset=0, current_session_id=NULL, current_turn_id=NULL,
                last_input=0, last_cached_input=0, last_output=0,
                last_reasoning=0, last_total=0, parse_error_count=0 WHERE path=?1",
        params![path],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn process_record(
    conn: &Connection,
    source_id: i64,
    file_path: &str,
    record_end: u64,
    record: LogRecord,
    cursor: &mut ScanCursor,
) -> AppResult<()> {
    let record_timestamp = record.timestamp.unwrap_or_else(db::now);
    let ordinal = record.ordinal.unwrap_or(record_end);
    match record.body {
        RecordBody::SessionMeta(meta) => {
            let session_id = meta
                .session_id
                .clone()
                .or_else(|| meta.id.clone())
                .or_else(|| session_id_from_path(file_path))
                .unwrap_or_else(|| format!("file:{file_path}"));
            upsert_session(
                conn,
                source_id,
                file_path,
                &session_id,
                &meta,
                &record_timestamp,
            )?;
            cursor.session_id = Some(session_id);
        }
        RecordBody::TurnContext(context) => {
            if let Some(turn_id) = context.turn_id.clone().or_else(|| cursor.turn_id.clone()) {
                let session_id = ensure_current_session(conn, source_id, file_path, cursor)?;
                ensure_turn(conn, source_id, &session_id, &turn_id, &record_timestamp)?;
                update_turn_context(conn, &turn_id, &context)?;
                cursor.turn_id = Some(turn_id);
            }
        }
        RecordBody::TokenUsageRecord(token) => {
            let Some(turn_id) = token.turn_id.clone().or_else(|| cursor.turn_id.clone()) else {
                return Ok(());
            };
            let session_id = token
                .session_id
                .clone()
                .or(token.thread_id.clone())
                .or_else(|| cursor.session_id.clone())
                .unwrap_or_else(|| format!("file:{file_path}"));
            let is_current = cursor.session_id.as_deref() == Some(session_id.as_str())
                && cursor.turn_id.as_deref() == Some(turn_id.as_str());
            if !is_current {
                ensure_placeholder_session(
                    conn,
                    source_id,
                    file_path,
                    &session_id,
                    &record_timestamp,
                )?;
                ensure_turn(conn, source_id, &session_id, &turn_id, &record_timestamp)?;
            }
            if let Some(usage) = token.usage.map(TokenUsage::non_negative) {
                let response_id = token
                    .response_id
                    .unwrap_or_else(|| format!("primary:{session_id}:{turn_id}:{ordinal}"));
                record_call(
                    conn,
                    &response_id,
                    &turn_id,
                    &session_id,
                    &record_timestamp,
                    "primary",
                    usage,
                )?;
            }
            cursor.session_id = Some(session_id);
            cursor.turn_id = Some(turn_id);
        }
        RecordBody::EventMsg(event) => match event {
            EventMessage::TaskStarted {
                turn_id,
                started_at,
            } => {
                if let Some(turn_id) = turn_id {
                    let session_id = ensure_current_session(conn, source_id, file_path, cursor)?;
                    let started = timestamp_from_seconds(started_at).unwrap_or(record_timestamp);
                    ensure_turn(conn, source_id, &session_id, &turn_id, &started)?;
                    cursor.turn_id = Some(turn_id);
                }
            }
            EventMessage::TaskComplete {
                turn_id,
                started_at,
                completed_at,
                duration_ms,
                time_to_first_token_ms,
                ..
            } => {
                if let Some(turn_id) = turn_id.or_else(|| cursor.turn_id.clone()) {
                    let session_id = ensure_current_session(conn, source_id, file_path, cursor)?;
                    let started =
                        timestamp_from_seconds(started_at).unwrap_or(record_timestamp.clone());
                    ensure_turn(conn, source_id, &session_id, &turn_id, &started)?;
                    let completed = timestamp_from_seconds(completed_at)
                        .unwrap_or_else(|| record_timestamp.clone());
                    conn.execute(
                        "UPDATE turns SET completed_at=?1, duration_ms=?2, ttft_ms=?3,
                                status='completed', updated_at=?4 WHERE turn_id=?5",
                        params![
                            completed,
                            duration_ms,
                            time_to_first_token_ms,
                            db::now(),
                            turn_id
                        ],
                    )
                    .map_err(db::to_error)?;
                    cursor.turn_id = Some(turn_id);
                }
            }
            EventMessage::TokenCount { info } => {
                if let (Some(turn_id), Some(total)) = (
                    cursor.turn_id.clone(),
                    info.and_then(|info| info.total_token_usage),
                ) {
                    let session_id = ensure_current_session(conn, source_id, file_path, cursor)?;
                    let total = total.non_negative();
                    let usage = total.delta_from(cursor.previous_total);
                    cursor.previous_total = total;
                    let response_id = format!("legacy:{session_id}:{turn_id}:{ordinal}");
                    record_call(
                        conn,
                        &response_id,
                        &turn_id,
                        &session_id,
                        &record_timestamp,
                        "legacy",
                        usage,
                    )?;
                }
            }
            EventMessage::Ignored => {}
        },
        RecordBody::Ignored => {}
    }
    Ok(())
}

fn ensure_current_session(
    conn: &Connection,
    source_id: i64,
    file_path: &str,
    cursor: &mut ScanCursor,
) -> AppResult<String> {
    if let Some(session_id) = cursor.session_id.clone() {
        return Ok(session_id);
    }
    let session_id = cursor
        .session_id
        .clone()
        .or_else(|| session_id_from_path(file_path))
        .unwrap_or_else(|| format!("file:{file_path}"));
    ensure_placeholder_session(conn, source_id, file_path, &session_id, &db::now())?;
    cursor.session_id = Some(session_id.clone());
    Ok(session_id)
}

fn ensure_placeholder_session(
    conn: &Connection,
    source_id: i64,
    file_path: &str,
    session_id: &str,
    timestamp: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sessions(session_id, source_id, file_path, started_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id) DO UPDATE SET source_id=excluded.source_id,
             file_path=excluded.file_path, updated_at=excluded.updated_at",
        params![session_id, source_id, file_path, timestamp, db::now()],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn upsert_session(
    conn: &Connection,
    source_id: i64,
    file_path: &str,
    session_id: &str,
    meta: &SessionMeta,
    fallback_timestamp: &str,
) -> AppResult<()> {
    let cwd = meta.cwd.clone().unwrap_or_default();
    let project = project_from_cwd(&cwd);
    let agent_kind = if meta.parent_thread_id.is_some()
        || meta.agent_path.is_some()
        || meta
            .thread_source
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("sub"))
    {
        "subagent"
    } else {
        "root"
    };
    conn.execute(
        "INSERT INTO sessions(session_id, source_id, file_path, parent_thread_id,
             thread_source, agent_path, agent_kind, cwd, project, started_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(session_id) DO UPDATE SET source_id=excluded.source_id,
             file_path=excluded.file_path, parent_thread_id=excluded.parent_thread_id,
             thread_source=excluded.thread_source, agent_path=excluded.agent_path,
             agent_kind=excluded.agent_kind, cwd=excluded.cwd, project=excluded.project,
             started_at=COALESCE(sessions.started_at, excluded.started_at),
             updated_at=excluded.updated_at",
        params![
            session_id,
            source_id,
            file_path,
            meta.parent_thread_id,
            meta.thread_source,
            meta.agent_path,
            agent_kind,
            cwd,
            project,
            meta.timestamp.as_deref().unwrap_or(fallback_timestamp),
            db::now()
        ],
    )
    .map_err(db::to_error)?;
    conn.execute(
        "UPDATE turns SET cwd=?1, project=?2 WHERE session_id=?3 AND cwd=''",
        params![cwd, project, session_id],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn ensure_turn(
    conn: &Connection,
    source_id: i64,
    session_id: &str,
    turn_id: &str,
    started_at: &str,
) -> AppResult<()> {
    let (cwd, project): (String, String) = conn
        .query_row(
            "SELECT cwd, project FROM sessions WHERE session_id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_default();
    conn.execute(
        "INSERT INTO turns(turn_id, session_id, source_id, started_at,
             started_local_date, cwd, project, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(turn_id) DO UPDATE SET session_id=excluded.session_id,
             source_id=excluded.source_id, updated_at=excluded.updated_at",
        params![
            turn_id,
            session_id,
            source_id,
            started_at,
            local_date(started_at),
            cwd,
            project,
            db::now()
        ],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn update_turn_context(conn: &Connection, turn_id: &str, context: &TurnContext) -> AppResult<()> {
    let cwd = context.cwd.clone().unwrap_or_default();
    let project = project_from_cwd(&cwd);
    conn.execute(
        "UPDATE turns SET model=COALESCE(?1, model), reasoning_effort=COALESCE(?2, reasoning_effort),
             cwd=CASE WHEN ?3='' THEN cwd ELSE ?3 END,
             project=CASE WHEN ?4='' THEN project ELSE ?4 END,
             updated_at=?5 WHERE turn_id=?6",
        params![context.model, context.effort, cwd, project, db::now(), turn_id],
    )
    .map_err(db::to_error)?;
    Ok(())
}

fn record_call(
    conn: &Connection,
    response_id: &str,
    turn_id: &str,
    session_id: &str,
    timestamp: &str,
    kind: &str,
    usage: TokenUsage,
) -> AppResult<()> {
    let has_primary = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM model_calls WHERE turn_id=?1 AND call_kind='primary')",
            params![turn_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(db::to_error)?;
    if kind == "legacy" && has_primary {
        return Ok(());
    }
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO model_calls(response_id, turn_id, session_id, occurred_at,
             call_kind, input_tokens, cached_input_tokens, output_tokens,
             reasoning_tokens, total_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                response_id,
                turn_id,
                session_id,
                timestamp,
                kind,
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens,
                usage.reasoning_output_tokens,
                usage.total_tokens
            ],
        )
        .map_err(db::to_error)?;
    if inserted == 0 {
        return Ok(());
    }
    if kind == "primary" && !has_primary {
        conn.execute(
            "UPDATE turns SET input_tokens=?1, cached_input_tokens=?2, output_tokens=?3,
                 reasoning_tokens=?4, total_tokens=?5, updated_at=?6 WHERE turn_id=?7",
            params![
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens,
                usage.reasoning_output_tokens,
                usage.total_tokens,
                db::now(),
                turn_id
            ],
        )
        .map_err(db::to_error)?;
    } else {
        conn.execute(
            "UPDATE turns SET input_tokens=input_tokens+?1,
                 cached_input_tokens=cached_input_tokens+?2,
                 output_tokens=output_tokens+?3,
                 reasoning_tokens=reasoning_tokens+?4,
                 total_tokens=total_tokens+?5, updated_at=?6 WHERE turn_id=?7",
            params![
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens,
                usage.reasoning_output_tokens,
                usage.total_tokens,
                db::now(),
                turn_id
            ],
        )
        .map_err(db::to_error)?;
    }
    Ok(())
}

fn mark_stale_turns(conn: &Connection, file_path: &str, mtime_ms: i64) -> AppResult<()> {
    let stale_before = Utc::now().timestamp_millis() - Duration::from_secs(600).as_millis() as i64;
    if mtime_ms < stale_before {
        conn.execute(
            "UPDATE turns SET status='incomplete'
             WHERE status='running' AND session_id IN
                 (SELECT session_id FROM sessions WHERE file_path=?1)",
            params![file_path],
        )
        .map_err(db::to_error)?;
    }
    Ok(())
}

fn timestamp_from_seconds(value: Option<f64>) -> Option<String> {
    let seconds = value?;
    let whole = seconds.floor() as i64;
    let nanos = ((seconds - seconds.floor()) * 1_000_000_000.0) as u32;
    DateTime::<Utc>::from_timestamp(whole, nanos).map(|value| value.to_rfc3339())
}

fn local_date(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| Local::now().format("%Y-%m-%d").to_string())
}

fn project_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(cwd)
        .to_string()
}

fn session_id_from_path(path: &str) -> Option<String> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    (stem.len() >= 36).then(|| stem[stem.len() - 36..].to_string())
}

/// Presents exactly one JSONL record as a `Read` stream without allocating the line.
struct JsonLineReader<'a, R: BufRead> {
    inner: &'a mut R,
    ended_by_newline: bool,
    done: bool,
}

impl<'a, R: BufRead> JsonLineReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            ended_by_newline: false,
            done: false,
        }
    }

    fn drain(&mut self) -> std::io::Result<()> {
        let mut buffer = [0_u8; 8192];
        while self.read(&mut buffer)? != 0 {}
        Ok(())
    }
}

impl<R: BufRead> Read for JsonLineReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.done || output.is_empty() {
            return Ok(0);
        }
        let available = self.inner.fill_buf()?;
        if available.is_empty() {
            self.done = true;
            return Ok(0);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        let copied = content_len.min(output.len());
        if copied > 0 {
            output[..copied].copy_from_slice(&available[..copied]);
            self.inner.consume(copied);
            return Ok(copied);
        }
        if newline == Some(0) {
            self.inner.consume(1);
            self.ended_by_newline = true;
            self.done = true;
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn line_reader_streams_without_crossing_record_boundary() {
        let mut source = BufReader::with_capacity(4, Cursor::new(b"{\"a\":1}\n{\"b\":2}\n"));
        {
            let mut first = JsonLineReader::new(&mut source);
            let value: serde_json::Value = serde_json::from_reader(&mut first).unwrap();
            first.drain().unwrap();
            assert_eq!(value["a"], 1);
            assert!(first.ended_by_newline);
        }
        let mut second = JsonLineReader::new(&mut source);
        let value: serde_json::Value = serde_json::from_reader(&mut second).unwrap();
        assert_eq!(value["b"], 2);
    }

    #[test]
    fn record_prefilter_skips_large_irrelevant_payloads() {
        assert!(!should_parse_record(
            br#"{"timestamp":"x","type":"response_item","payload":{"type":"message"}}"#
        ));
        assert!(!should_parse_record(
            br#"{"timestamp":"x","type":"event_msg","payload":{"type":"item_completed"}}"#
        ));
        assert!(should_parse_record(
            br#"{"timestamp":"x", "type" : "event_msg", "payload":{"type":"task_complete"}}"#
        ));
        assert!(should_parse_record(
            br#"{"timestamp":"x","type":"token_usage_record","payload":{}}"#
        ));
    }

    #[test]
    fn fast_selected_records_stop_before_large_private_fields() {
        let session = br#"{"timestamp":"2026-09-05T00:00:00Z","ordinal":1,"type":"session_meta","payload":{"id":"s1","parent_thread_id":"root","cwd":"/tmp/demo","agent_path":"/root/worker","thread_source":"subagent","timestamp":"2026-09-05T00:00:00Z","base_instructions":{"session_id":"private-collision","text":"private"}}}"#;
        let Some(LogRecord {
            body: RecordBody::SessionMeta(meta),
            ..
        }) = fast_selected_record(session)
        else {
            panic!("session")
        };
        assert_eq!(meta.session_id, None);
        assert_eq!(meta.id.as_deref(), Some("s1"));
        assert_eq!(meta.parent_thread_id.as_deref(), Some("root"));

        let context = br#"{"timestamp":"2026-09-05T00:00:01Z","ordinal":2,"type":"turn_context","payload":{"turn_id":"t1","cwd":"/tmp/demo","model":"gpt-test","effort":"high","summary":{"text":"private"}}}"#;
        let Some(LogRecord {
            body: RecordBody::TurnContext(turn),
            ..
        }) = fast_selected_record(context)
        else {
            panic!("turn")
        };
        assert_eq!(turn.turn_id.as_deref(), Some("t1"));
        assert_eq!(turn.model.as_deref(), Some("gpt-test"));
        assert_eq!(turn.effort.as_deref(), Some("high"));
    }

    #[test]
    fn cumulative_usage_delta_handles_resets() {
        let previous = TokenUsage {
            input_tokens: 100,
            output_tokens: 20,
            total_tokens: 120,
            ..TokenUsage::default()
        };
        let current = TokenUsage {
            input_tokens: 130,
            output_tokens: 30,
            total_tokens: 160,
            ..TokenUsage::default()
        };
        assert_eq!(current.delta_from(previous).input_tokens, 30);
        assert_eq!(previous.delta_from(current).input_tokens, 100);
    }

    #[test]
    fn indexes_selective_metrics_and_deduplicates_moved_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("meter.sqlite3");
        db::migrate(&database).unwrap();
        let conn = db::open(&database).unwrap();
        conn.execute(
            "INSERT INTO sources(name, root_path, enabled) VALUES ('Test', ?1, 1)",
            params![temp.path().to_string_lossy()],
        )
        .unwrap();
        let source_id = conn.last_insert_rowid();
        let body = concat!(
            "{\"timestamp\":\"2026-09-05T00:00:00Z\",\"type\":\"session_meta\",\"ordinal\":1,\"payload\":{\"session_id\":\"session-a\",\"id\":\"session-a\",\"cwd\":\"/tmp/project-a\",\"base_instructions\":{\"secret\":\"discard\"}}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:01Z\",\"type\":\"event_msg\",\"ordinal\":2,\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-a\",\"started_at\":1788566401}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:02Z\",\"type\":\"turn_context\",\"ordinal\":3,\"payload\":{\"turn_id\":\"turn-a\",\"model\":\"gpt-test\",\"effort\":\"high\",\"summary\":\"discard\"}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:03Z\",\"type\":\"event_msg\",\"ordinal\":4,\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":20,\"reasoning_output_tokens\":5,\"total_tokens\":120}}}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:03Z\",\"type\":\"token_usage_record\",\"ordinal\":5,\"payload\":{\"session_id\":\"session-a\",\"turn_id\":\"turn-a\",\"response_id\":\"response-a\",\"usage\":{\"input_tokens\":80,\"cached_input_tokens\":30,\"output_tokens\":10,\"reasoning_output_tokens\":4,\"total_tokens\":90}}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:04Z\",\"type\":\"event_msg\",\"ordinal\":6,\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"turn-a\",\"started_at\":1788566401,\"completed_at\":1788566404,\"duration_ms\":3000,\"time_to_first_token_ms\":1000,\"last_agent_message\":\"discard\"}}\n"
        );
        let first_path = temp.path().join("rollout-first.jsonl");
        std::fs::write(&first_path, body).unwrap();
        let first = test_file(source_id, first_path);
        let state = BackendState::new(database.clone());
        scan_file(&conn, &first, &state, |_| {}).unwrap();

        let overview = db::query_overview(&database, Default::default()).unwrap();
        assert_eq!(overview.turn_count, 1);
        assert_eq!(overview.tokens.input, 80);
        assert_eq!(overview.tokens.cached_input, 30);
        assert_eq!(overview.tokens.total, 90);
        assert_eq!(overview.median_effective_tps, Some(5.0));

        let moved_path = temp.path().join("rollout-moved.jsonl");
        std::fs::write(&moved_path, body).unwrap();
        let moved = test_file(source_id, moved_path);
        scan_file(&conn, &moved, &state, |_| {}).unwrap();
        let overview = db::query_overview(&database, Default::default()).unwrap();
        assert_eq!(overview.turn_count, 1);
        assert_eq!(overview.tokens.total, 90);
    }

    #[test]
    fn skips_bad_record_and_retries_incomplete_tail() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("meter.sqlite3");
        db::migrate(&database).unwrap();
        let conn = db::open(&database).unwrap();
        conn.execute(
            "INSERT INTO sources(name, root_path, enabled) VALUES ('Test', ?1, 1)",
            params![temp.path().to_string_lossy()],
        )
        .unwrap();
        let source_id = conn.last_insert_rowid();
        let complete_prefix = concat!(
            "{\"timestamp\":\"2026-09-05T00:00:00Z\",\"type\":\"session_meta\",\"ordinal\":1,\"payload\":{\"session_id\":\"session-partial\"}}\n",
            "{\"timestamp\":\"2026-09-05T00:00:01Z\",\"type\":\"event_msg\",\"ordinal\":2,\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-partial\"}}\n",
            "this is a malformed record\n"
        );
        let partial = "{\"timestamp\":\"2026-09-05T00:00:02Z\",\"type\":\"token_usage_record\",\"ordinal\":3,\"payload\":{\"session_id\":\"session-partial\",\"turn_id\":\"turn-partial\",\"response_id\":\"response-partial\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3";
        let path = temp.path().join("rollout-partial.jsonl");
        std::fs::write(&path, format!("{complete_prefix}{partial}")).unwrap();
        let state = BackendState::new(database.clone());
        let first_offset =
            scan_file(&conn, &test_file(source_id, path.clone()), &state, |_| {}).unwrap();
        assert_eq!(first_offset, complete_prefix.len() as u64);

        use std::io::Write;
        let mut output = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        output.write_all(b"}}}\n").unwrap();
        drop(output);
        let final_offset = scan_file(&conn, &test_file(source_id, path), &state, |_| {}).unwrap();
        assert!(final_offset > first_offset);
        let overview = db::query_overview(&database, Default::default()).unwrap();
        assert_eq!(overview.tokens.total, 3);
    }

    fn test_file(source_id: i64, path: PathBuf) -> FileEntry {
        let metadata = std::fs::metadata(&path).unwrap();
        FileEntry {
            source_id,
            path,
            size: metadata.len(),
            mtime_ms: Utc::now().timestamp_millis(),
        }
    }
}
