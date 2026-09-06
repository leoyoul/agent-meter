use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricFilters {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub source_id: Option<i64>,
    pub model: Option<String>,
    pub project: Option<String>,
    pub agent_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MetricPeriod {
    #[default]
    Realtime,
    Today,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsFilters {
    #[serde(default)]
    pub period: MetricPeriod,
    pub source_id: Option<i64>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub uncached_input: i64,
    pub cached_read: i64,
    pub cached_write: i64,
    pub output: i64,
    pub reasoning: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingCoverage {
    pub priced_observations: i64,
    pub total_observations: i64,
    pub priced_tokens: i64,
    pub total_tokens: i64,
    pub ratio: f64,
    pub complete: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSummary {
    pub period: MetricPeriod,
    pub observation_count: i64,
    pub average_ttft_ms: Option<f64>,
    pub average_effective_tps: Option<f64>,
    pub tokens: UsageTokens,
    pub estimated_cost_nano_usd: i64,
    pub pricing: PricingCoverage,
    pub last_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSeriesPoint {
    pub bucket: String,
    pub label: String,
    pub observation_count: i64,
    pub average_ttft_ms: Option<f64>,
    pub average_effective_tps: Option<f64>,
    pub total_tokens: i64,
    pub estimated_cost_nano_usd: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEffortStat {
    pub source_id: i64,
    pub source_name: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: String,
    pub observation_count: i64,
    pub average_ttft_ms: Option<f64>,
    pub average_effective_tps: Option<f64>,
    pub tokens: UsageTokens,
    pub estimated_cost_nano_usd: i64,
    pub pricing: PricingCoverage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingRate {
    pub vendor: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub currency: String,
    pub input_usd_per_million: String,
    pub cached_read_usd_per_million: Option<String>,
    pub cached_write_usd_per_million: Option<String>,
    pub output_usd_per_million: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub source_url: String,
    pub verified_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingCatalogStatus {
    pub version: String,
    pub currency: String,
    pub verified_at: String,
    pub rates: Vec<PricingRate>,
    pub priced_observations: i64,
    pub total_observations: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub enabled: bool,
    pub available: bool,
    pub file_count: u64,
    pub total_bytes: u64,
    pub last_scan_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    pub running: bool,
    pub paused: bool,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_file: Option<String>,
    pub started_at: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBreakdown {
    pub input: i64,
    pub cached_input: i64,
    pub output: i64,
    pub reasoning: i64,
    pub total: i64,
}

impl std::ops::AddAssign for TokenBreakdown {
    fn add_assign(&mut self, other: Self) {
        self.input += other.input;
        self.cached_input += other.cached_input;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.total += other.total;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub tokens: TokenBreakdown,
    pub turn_count: i64,
    pub running_count: i64,
    pub subagent_count: i64,
    pub median_ttft_ms: Option<f64>,
    pub p95_ttft_ms: Option<f64>,
    pub median_duration_ms: Option<f64>,
    pub p95_duration_ms: Option<f64>,
    pub median_effective_tps: Option<f64>,
    pub recent_median_ttft_ms: Option<f64>,
    pub recent_median_effective_tps: Option<f64>,
    pub last_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeseriesPoint {
    pub bucket: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub median_ttft_ms: Option<f64>,
    pub median_effective_tps: Option<f64>,
    pub turn_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub turn_count: i64,
    pub tokens: TokenBreakdown,
    pub median_ttft_ms: Option<f64>,
    pub p95_ttft_ms: Option<f64>,
    pub median_duration_ms: Option<f64>,
    pub p95_duration_ms: Option<f64>,
    pub median_effective_tps: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub turn_id: String,
    pub session_id: String,
    pub parent_thread_id: Option<String>,
    pub source_name: String,
    pub project: String,
    pub cwd: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub agent_kind: String,
    pub agent_path: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub effective_tps: Option<f64>,
    pub tokens: TokenBreakdown,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct LogRecord {
    pub timestamp: Option<String>,
    pub ordinal: Option<u64>,
    #[serde(flatten)]
    pub body: RecordBody,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RecordBody {
    #[serde(rename = "session_meta")]
    SessionMeta(SessionMeta),
    #[serde(rename = "turn_context")]
    TurnContext(TurnContext),
    #[serde(rename = "token_usage_record")]
    TokenUsageRecord(TokenUsageRecord),
    #[serde(rename = "event_msg")]
    EventMsg(EventMessage),
    #[serde(other)]
    Ignored,
}

#[derive(Debug, Default, Deserialize)]
pub struct SessionMeta {
    pub session_id: Option<String>,
    pub id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub thread_source: Option<String>,
    pub agent_path: Option<String>,
    pub cwd: Option<String>,
    pub timestamp: Option<String>,
    #[serde(default, rename = "base_instructions")]
    _base_instructions: Option<IgnoredAny>,
}

impl SessionMeta {
    pub fn selected(
        session_id: Option<String>,
        id: Option<String>,
        parent_thread_id: Option<String>,
        thread_source: Option<String>,
        agent_path: Option<String>,
        cwd: Option<String>,
        timestamp: Option<String>,
    ) -> Self {
        Self {
            session_id,
            id,
            parent_thread_id,
            thread_source,
            agent_path,
            cwd,
            timestamp,
            _base_instructions: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TurnContext {
    pub turn_id: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, rename = "summary")]
    _summary: Option<IgnoredAny>,
}

impl TurnContext {
    pub fn selected(
        turn_id: Option<String>,
        cwd: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    ) -> Self {
        Self {
            turn_id,
            cwd,
            model,
            effort,
            _summary: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenUsageRecord {
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub response_id: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub cached_input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub reasoning_output_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

impl TokenUsage {
    pub fn non_negative(self) -> Self {
        Self {
            input_tokens: self.input_tokens.max(0),
            cached_input_tokens: self.cached_input_tokens.max(0),
            output_tokens: self.output_tokens.max(0),
            reasoning_output_tokens: self.reasoning_output_tokens.max(0),
            total_tokens: self.total_tokens.max(0),
        }
    }

    pub fn delta_from(self, previous: Self) -> Self {
        let delta = Self {
            input_tokens: self.input_tokens - previous.input_tokens,
            cached_input_tokens: self.cached_input_tokens - previous.cached_input_tokens,
            output_tokens: self.output_tokens - previous.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens
                - previous.reasoning_output_tokens,
            total_tokens: self.total_tokens - previous.total_tokens,
        };
        if delta.input_tokens < 0 || delta.output_tokens < 0 || delta.total_tokens < 0 {
            self.non_negative()
        } else {
            delta.non_negative()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum EventMessage {
    #[serde(rename = "task_started")]
    TaskStarted {
        turn_id: Option<String>,
        started_at: Option<f64>,
    },
    #[serde(rename = "task_complete")]
    TaskComplete {
        turn_id: Option<String>,
        started_at: Option<f64>,
        completed_at: Option<f64>,
        duration_ms: Option<i64>,
        time_to_first_token_ms: Option<i64>,
        #[serde(default, rename = "last_agent_message")]
        _last_agent_message: Option<IgnoredAny>,
    },
    #[serde(rename = "token_count")]
    TokenCount { info: Option<TokenCountInfo> },
    #[serde(other)]
    Ignored,
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenCountInfo {
    pub total_token_usage: Option<TokenUsage>,
}
