use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Tokens {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    // A subset of cache_write_input_tokens, never additional input tokens.
    #[serde(default)]
    pub cache_write_12h_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

impl Tokens {
    pub fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
    pub fn delta(self, prior: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(prior.input_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_sub(prior.cached_input_tokens),
            cache_write_input_tokens: self
                .cache_write_input_tokens
                .saturating_sub(prior.cache_write_input_tokens),
            cache_write_12h_input_tokens: self
                .cache_write_12h_input_tokens
                .saturating_sub(prior.cache_write_12h_input_tokens),
            output_tokens: self.output_tokens.saturating_sub(prior.output_tokens),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub id: String,
    pub at: i64,
    pub thread: String,
    pub turn: String,
    pub root_turn: String,
    pub parent: Option<String>,
    pub project: String,
    pub model: String,
    pub service_tier: Option<String>,
    pub request_mode: bool,
    pub is_subagent: bool,
    pub tokens: Tokens,
    pub modern: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Turn {
    pub id: String,
    pub thread: String,
    pub root_turn: String,
    pub project: String,
    pub title: String,
    pub started: i64,
    pub completed: Option<i64>,
    pub is_subagent: bool,
}

#[derive(Debug, Clone)]
pub struct ResetWindow {
    pub end: i64,
    pub seconds: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub service_tiers: Vec<ServiceTierMetrics>,
    pub request_mode_tokens: u64,
    pub assumed_standard_tokens: u64,
    pub unpriced_tokens: u64,
    pub main_agent_count: usize,
    pub sub_agent_count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTierMetrics {
    pub tier: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub request_mode_tokens: u64,
    pub assumed_standard_tokens: u64,
    pub unpriced_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub start: i64,
    pub end: i64,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub id: String,
    pub name: String,
    pub kind: ShareKind,
    pub tokens: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareKind {
    Model,
    Project,
    Conversation,
    Unassigned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cycle {
    pub id: String,
    pub start: i64,
    pub end: i64,
    pub ongoing: bool,
    pub estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleDetails {
    pub cycle: Cycle,
    pub metrics: Metrics,
    pub models: Vec<Share>,
    pub attributions: Vec<Share>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageComparisons {
    pub previous: Metrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub id: String,
    pub title: String,
    pub project: String,
    pub project_id: Option<String>,
    pub thread_id: String,
    pub thread_title: String,
    pub models: Vec<String>,
    pub started: i64,
    pub completed: i64,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub thread_id: String,
    pub title: String,
    pub project: String,
    pub project_id: Option<String>,
    pub models: Vec<String>,
    pub execution_count: usize,
    pub started: i64,
    pub last_completed: i64,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub files: usize,
    pub unreadable_files: usize,
    pub malformed_lines: usize,
    pub oversized_lines: usize,
    pub first_event_at: Option<i64>,
    pub last_event_at: Option<i64>,
    pub unassigned_events: usize,
    pub excluded_internal_events: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub hours: u32,
    pub cycle_id: Option<String>,
    pub project: Option<String>,
    pub billing_start: Option<i64>,
    pub execution_hours: Option<u32>,
    pub project_scope: Option<String>,
    pub thread_id: Option<String>,
    pub page: Option<usize>,
    pub trend_before: Option<i64>,
    pub refresh_pricing: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilterOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPage {
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub updated_at: i64,
    pub start: i64,
    pub end: i64,
    pub history: Metrics,
    pub comparisons: UsageComparisons,
    pub trend: Vec<Bucket>,
    pub trend_bucket_seconds: i64,
    pub trend_has_older: bool,
    pub billing_start: i64,
    pub billing_is_rolling: bool,
    pub billing: Metrics,
    pub cycles: Vec<Cycle>,
    pub cycle: Option<CycleDetails>,
    pub executions: Vec<Execution>,
    pub projects: Vec<FilterOption>,
    pub conversations: Vec<FilterOption>,
    pub execution_page: ExecutionPage,
    pub sessions: Vec<ConversationSummary>,
    pub session_page: ExecutionPage,
    pub coverage: Coverage,
    pub pricing_date: String,
    pub pricing: super::pricing::PricingStatus,
}
