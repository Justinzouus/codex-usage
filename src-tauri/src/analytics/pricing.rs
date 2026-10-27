use super::types::{Tokens, UsageEvent};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

pub const SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing";
const SOURCE_MARKDOWN: &str = "https://developers.openai.com/api/docs/pricing.md";
const BUNDLED_DATE: &str = "2026-10-13";
const MAX_BYTES: usize = 1_048_576;
const DEBOUNCE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingModel {
    pub model: String,
    pub tokens: u64,
    pub reason: String,
    pub source_url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingStatus {
    pub status: String,
    pub last_checked_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub source_url: String,
    pub using_cached_rates: bool,
    pub error: Option<String>,
    pub missing_models: Vec<MissingModel>,
    pub model_count: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Rates {
    input: f64,
    cached: Option<f64>,
    write: Option<f64>,
    #[serde(default)]
    write_12h: Option<f64>,
    output: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ModelRates {
    short: Rates,
    long: Option<Rates>,
    threshold: Option<u64>,
    source_url: String,
    verified_date: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateCard {
    models: BTreeMap<String, ModelRates>,
    tiers: BTreeMap<String, BTreeMap<String, ModelRates>>,
    pub date: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingSnapshot {
    version: u32,
    pub card: RateCard,
    pub status: PricingStatus,
    lookup_errors: BTreeMap<String, LookupError>,
    #[serde(default)]
    retained_rates: BTreeMap<String, String>,
    #[serde(default)]
    main_error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LookupError {
    checked_at: i64,
    reason: String,
}

fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 100
        && model.as_bytes()[0].is_ascii_alphanumeric()
        && model.as_bytes()[model.len() - 1].is_ascii_alphanumeric()
        && model
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-._".contains(&c))
}
pub fn model_url(model: &str) -> String {
    if valid_model(model) {
        format!("https://developers.openai.com/api/docs/models/{model}")
    } else {
        SOURCE_URL.into()
    }
}
pub fn is_pricing_url(url: &str) -> bool {
    url == SOURCE_URL
        || url
            .strip_prefix("https://developers.openai.com/api/docs/models/")
            .is_some_and(valid_model)
}
pub fn normalize_tier(tier: Option<&str>) -> String {
    match tier.map(str::trim).filter(|s| !s.is_empty()) {
        None => "unknown",
        Some("default" | "standard") => "standard",
        Some("priority" | "fast") => "fast",
        Some(other) => other,
    }
    .to_string()
}
impl Default for PricingSnapshot {
    fn default() -> Self {
        Self::bundled()
    }
}
impl RateCard {
    fn lookup(&self, model: &str) -> Option<&ModelRates> {
        // Published snapshots can have different rates; prefer the exact ID.
        self.models.get(model.trim())
    }
    #[cfg(test)]
    pub fn estimate(&self, model: &str, tokens: Tokens) -> Result<f64, &'static str> {
        self.estimate_tier(model, tokens, None)
    }
    pub fn estimate_tier(
        &self,
        model: &str,
        tokens: Tokens,
        tier: Option<&str>,
    ) -> Result<f64, &'static str> {
        let tier = normalize_tier(tier);
        let rate = if tier == "unknown" || tier == "standard" {
            self.lookup(model)
        } else {
            self.tiers
                .get(&tier)
                .and_then(|models| models.get(model.trim()))
        }
        .ok_or(if tier == "unknown" || tier == "standard" {
            "官方价格表未包含此模型，尚未取得可验证的 API 价格"
        } else {
            "官方未公布此模型对应服务模式的 API 价格，不能套用 Standard 价格"
        })?;
        if tokens
            .cached_input_tokens
            .checked_add(tokens.cache_write_input_tokens)
            .is_none_or(|n| n > tokens.input_tokens)
        {
            return Err("本地记录的缓存读取与写入 token 超出输入总数，无法可靠估价");
        }
        if tokens.cache_write_12h_input_tokens > tokens.cache_write_input_tokens {
            return Err("本地记录的 12h 缓存写入 token 超出缓存写入总数，无法可靠估价");
        }
        let r = if rate
            .threshold
            .is_some_and(|threshold| tokens.input_tokens > threshold)
        {
            rate.long.as_ref().ok_or("官方未公布此模型的长上下文价格")?
        } else {
            &rate.short
        };
        if tokens.cached_input_tokens > 0 && r.cached.is_none() {
            return Err("官方未公布此模型的缓存输入价格");
        }
        let write_30m = tokens.cache_write_input_tokens - tokens.cache_write_12h_input_tokens;
        if write_30m > 0 && r.write.is_none() {
            return Err("官方未公布此模型的缓存写入价格");
        }
        if tokens.cache_write_12h_input_tokens > 0 && r.write_12h.is_none() {
            return Err("官方未公布此模型的 12h 缓存写入价格");
        }
        let ordinary =
            tokens.input_tokens - tokens.cached_input_tokens - tokens.cache_write_input_tokens;
        Ok((ordinary as f64 * r.input
            + tokens.cached_input_tokens as f64 * r.cached.unwrap_or(0.0)
            + write_30m as f64 * r.write.unwrap_or(0.0)
            + tokens.cache_write_12h_input_tokens as f64 * r.write_12h.unwrap_or(0.0)
            + tokens.output_tokens as f64 * r.output)
            / 1_000_000.0)
    }
    fn validate(&self) -> bool {
        !self.models.is_empty()
            && self.models.len() <= 2000
            && self
                .models
                .iter()
                .chain(self.tiers.values().flat_map(|v| v.iter()))
                .all(|(id, m)| {
                    valid_model(id)
                        && is_pricing_url(&m.source_url)
                        && (!m.long.is_some() || m.threshold.is_some())
                        && m.threshold.is_none_or(|n| n > 0)
                        && [&m.short].into_iter().chain(m.long.as_ref()).all(|r| {
                            [
                                Some(r.input),
                                r.cached,
                                r.write,
                                r.write_12h,
                                Some(r.output),
                            ]
                            .into_iter()
                            .flatten()
                            .all(|v| v.is_finite() && (0.0..=1_000_000.0).contains(&v))
                        })
                })
    }
}
impl PricingSnapshot {
    pub fn bundled() -> Self {
        let card = bundled_card().clone();
        Self {
            version: 1,
            lookup_errors: BTreeMap::new(),
            retained_rates: BTreeMap::new(),
            main_error: None,
            status: PricingStatus {
                status: "neverChecked".into(),
                last_checked_at: None,
                last_success_at: None,
                source_url: SOURCE_URL.into(),
                using_cached_rates: true,
                error: None,
                missing_models: vec![],
                model_count: card.models.len(),
            },
            card,
        }
    }
    pub fn status_for(&self, events: &[UsageEvent]) -> PricingStatus {
        let mut status = self.status.clone();
        let mut missing: BTreeMap<(String, String), u64> = BTreeMap::new();
        for e in events {
            if let Err(reason) =
                self.card
                    .estimate_tier(&e.model, e.tokens, e.service_tier.as_deref())
            {
                let model = if e.model.is_empty() {
                    "未知模型".into()
                } else {
                    e.model.clone()
                };
                let tokens = missing
                    .entry((
                        model.clone(),
                        self.lookup_errors
                            .get(&model)
                            .map(|e| format!("{reason}；{}", e.reason))
                            .unwrap_or_else(|| reason.into()),
                    ))
                    .or_default();
                *tokens = tokens.saturating_add(e.tokens.total());
            }
        }
        status.missing_models = missing
            .into_iter()
            .map(|((model, reason), tokens)| MissingModel {
                source_url: model_url(&model),
                model,
                tokens,
                reason,
            })
            .collect();
        status
            .missing_models
            .sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.model.cmp(&b.model)));
        status.model_count = self.card.models.len();
        status
    }
}
fn bundled_card() -> &'static RateCard {
    static CARD: OnceLock<RateCard> = OnceLock::new();
    CARD.get_or_init(|| {
        parse_standard(
            include_str!("fixtures/official-pricing-2026-10-13.md"),
            BUNDLED_DATE,
        )
        .expect("verified bundled pricing table")
    })
}
#[cfg(test)]
pub fn estimate(model: &str, tokens: Tokens) -> Option<f64> {
    bundled_card().estimate(model, tokens).ok()
}

fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}
fn price(text: &str) -> Result<Option<f64>, String> {
    if text == "-" {
        return Ok(None);
    }
    let n: f64 = text
        .strip_prefix('$')
        .ok_or_else(|| format!("非预期的价格格式：{text}"))?
        .parse()
        .map_err(|_| "价格不是有效数字")?;
    if !n.is_finite() || !(0.0..=1_000_000.0).contains(&n) {
        return Err("价格超出有效范围".into());
    }
    Ok(Some(n))
}
fn context_threshold(markdown: &str) -> Result<u64, String> {
    let prefix = "Short context: ≤";
    let value = markdown
        .split_once(prefix)
        .and_then(|(_, v)| v.split_once(" input tokens. Long context: >"))
        .ok_or("官方长上下文分界说明格式已变化")?;
    let end = value
        .1
        .split_once(" input tokens.")
        .ok_or("缺少长上下文分界说明")?
        .0;
    if value.0 != end {
        return Err("上下文分界说明不一致".into());
    }
    let (num, mult) = if let Some(num) = end.strip_suffix('K') {
        (num, 1000)
    } else {
        (end, 1)
    };
    num.parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
        .filter(|n| *n > 0)
        .ok_or_else(|| "无法解析上下文分界值".into())
}
/// Column names, rather than positions, define prices. The two cache-write
/// durations are separate fields; an unknown column fails a text-price table.
struct RateColumns {
    input: usize,
    cached: usize,
    write: Option<usize>,
    write_12h: Option<usize>,
    output: usize,
}
impl RateColumns {
    fn new(columns: &[&str], prefix: &str, context: bool) -> Result<Self, String> {
        let index = |name: &str| {
            columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(&format!("{prefix}{name}")))
        };
        let required =
            |name: &str| index(name).ok_or_else(|| format!("价格表缺少列：{prefix}{name}"));
        let legacy_write = index("Cache writes");
        let write_30m = index("30m write");
        if legacy_write.is_some() && write_30m.is_some() {
            return Err("价格表同时包含旧缓存写入列和 30m 写入列，计价含义不明确".into());
        }
        let write = legacy_write.or(write_30m);
        if context && write.is_none() {
            return Err(format!("价格表缺少列：{prefix}Cache writes / 30m write"));
        }
        Ok(Self {
            input: required("Input")?,
            cached: required("Cached input")?,
            write,
            write_12h: index("12h write"),
            output: required("Output")?,
        })
    }
    fn read(&self, row: &[&str], optional: bool) -> Result<Option<Rates>, String> {
        let input = price(row[self.input])?;
        let cached = price(row[self.cached])?;
        let write = self.write.map(|i| price(row[i])).transpose()?.flatten();
        let write_12h = self.write_12h.map(|i| price(row[i])).transpose()?.flatten();
        let output = price(row[self.output])?;
        if optional
            && [input, cached, write, write_12h, output]
                .iter()
                .all(Option::is_none)
        {
            return Ok(None);
        }
        Ok(Some(Rates {
            input: input.ok_or("缺少输入价格")?,
            cached,
            write,
            write_12h,
            output: output.ok_or("缺少输出价格")?,
        }))
    }
}

fn merge_model(
    models: &mut BTreeMap<String, ModelRates>,
    id: &str,
    rate: ModelRates,
) -> Result<(), String> {
    if let Some(previous) = models.get(id) {
        if previous != &rate {
            return Err(format!(
                "官方同一服务模式的模型 {id} 出现冲突价格，已保留上次价格"
            ));
        }
    } else {
        models.insert(id.into(), rate);
    }
    Ok(())
}

fn parse_text_table(
    lines: &[&str],
    markdown: &str,
    models: &mut BTreeMap<String, ModelRates>,
) -> Result<usize, String> {
    let columns = cells(lines[0]);
    let lowered: Vec<_> = columns.iter().map(|c| c.to_ascii_lowercase()).collect();
    let unique: std::collections::BTreeSet<_> = lowered.iter().collect();
    if unique.len() != columns.len() {
        return Err("价格表出现重复列".into());
    }
    let model_index = columns
        .iter()
        .position(|c| *c == "Model")
        .ok_or("价格表缺少 Model 列")?;
    let category_index = columns.iter().position(|c| *c == "Category");
    let context = columns.iter().any(|c| c.starts_with("Short context "));
    let allowed = [
        "input",
        "cached input",
        "cache writes",
        "30m write",
        "12h write",
        "output",
    ];
    for column in &lowered {
        if column == "model" || column == "category" {
            continue;
        }
        let name = if context {
            column
                .strip_prefix("short context ")
                .or_else(|| column.strip_prefix("long context "))
        } else {
            Some(column.as_str())
        };
        if !name.is_some_and(|name| allowed.contains(&name)) {
            return Err(format!("价格表增加了未知计价列：{column}"));
        }
    }
    let short = RateColumns::new(
        &columns,
        if context { "Short context " } else { "" },
        context,
    )?;
    let long = context
        .then(|| RateColumns::new(&columns, "Long context ", true))
        .transpose()?;
    let threshold = context.then(|| context_threshold(markdown)).transpose()?;
    // The new duration-aware context table contains only context-priced models;
    // unbounded prices are published in a separate plain table. Legacy tables
    // mixed both kinds, so keep their explicit qualifier/long-rate behavior.
    let explicit_context_range = context && short.write_12h.is_some();
    let separator = cells(lines.get(1).ok_or("价格表缺少分隔行")?);
    if separator.len() != columns.len()
        || !separator
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|b| b"-:".contains(&b)))
    {
        return Err("价格表分隔行格式异常".into());
    }
    let mut count = 0;
    for line in &lines[2..] {
        let row = cells(line);
        if row.len() != columns.len() {
            return Err("价格表存在不完整行".into());
        }
        // Specialized tables also list embeddings, moderation and other units.
        // Only rows whose complete text input/output price is used here qualify.
        if category_index.is_some_and(|i| !["ChatGPT", "Codex", "Search"].contains(&row[i])) {
            continue;
        }
        let qualified = row[model_index].ends_with(" (<272K context length)");
        if qualified && threshold != Some(272_000) {
            return Err("模型与总表的上下文边界不一致".into());
        }
        let id = row[model_index]
            .strip_suffix(" (<272K context length)")
            .unwrap_or(row[model_index]);
        if !valid_model(id) {
            return Err(format!("价格表存在无法识别的模型名称：{id}"));
        }
        let short = short.read(&row, false)?.ok_or("缺少短上下文价格")?;
        let long = long
            .as_ref()
            .map(|c| c.read(&row, true))
            .transpose()?
            .flatten();
        let rate = ModelRates {
            threshold: (long.is_some() || qualified || explicit_context_range)
                .then_some(threshold)
                .flatten(),
            short,
            long,
            source_url: SOURCE_URL.into(),
            verified_date: String::new(),
        };
        merge_model(models, id, rate)?;
        count += 1;
    }
    if count == 0 {
        return Err("文本价格表为空".into());
    }
    Ok(count)
}

fn parse_standard(markdown: &str, date: &str) -> Result<RateCard, String> {
    let mut tables: BTreeMap<String, BTreeMap<String, ModelRates>> = BTreeMap::new();
    let lines: Vec<_> = markdown.lines().map(str::trim).collect();
    let mut mode: Option<String> = None;
    let mut group_mode = "standard".to_string();
    let mut explicit = false;
    let mut section_rows = 0;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(name) = line
            .strip_prefix("### ")
            .and_then(|s| s.strip_suffix(" pricing data"))
        {
            if explicit && section_rows == 0 {
                return Err("官方服务模式价格表为空".into());
            }
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err("官方服务模式名称无法识别".into());
            }
            mode = Some(name.to_ascii_lowercase());
            explicit = true;
            section_rows = 0;
        } else if let Some(name) = line.strip_prefix("#### ").filter(|_| explicit) {
            // Batch's page groups distinct Batch and Flex tables under one heading.
            if !["Standard", "Batch", "Flex", "Fast", "Ultrafast"].contains(&name) {
                return Err("官方价格表子模式无法识别".into());
            }
            mode = Some(name.to_ascii_lowercase());
        } else if line.starts_with('#') {
            if explicit && section_rows == 0 {
                return Err("官方服务模式价格表为空".into());
            }
            explicit = false;
            section_rows = 0;
            // Component titles describe table layout, never a service tier.
            mode = if ["### Grouped Pricing Table data", "### Pricing Table data"].contains(&line) {
                Some(group_mode.clone())
            } else {
                None
            };
        } else if ["Standard", "Batch", "Flex", "Fast", "Ultrafast"].contains(&line) {
            if explicit && section_rows == 0 {
                return Err("官方服务模式价格表为空".into());
            }
            explicit = false;
            group_mode = line.to_ascii_lowercase();
            mode = Some(group_mode.clone());
        } else if line.ends_with(" models") || line == "Finetuning" || line == "Tools" {
            if explicit && section_rows == 0 {
                return Err("官方服务模式价格表为空".into());
            }
            explicit = false;
            mode = None;
            group_mode = "standard".into();
        } else if line.starts_with('|') {
            let end = (i..lines.len())
                .find(|n| !lines[*n].starts_with('|'))
                .unwrap_or(lines.len());
            if let Some(mode) = &mode {
                let columns = cells(line);
                let simple_text = columns.iter().all(|c| {
                    [
                        "Category",
                        "Model",
                        "Input",
                        "Output",
                        "Cached input",
                        "Cache writes",
                        "30m write",
                        "12h write",
                    ]
                    .contains(c)
                }) && ["Model", "Input", "Output", "Cached input"]
                    .iter()
                    .all(|c| columns.contains(c));
                // Modality, per-minute, tools and training tables are intentionally
                // outside text-token accounting, even when the heading is generic.
                if explicit || simple_text {
                    section_rows += parse_text_table(
                        &lines[i..end],
                        markdown,
                        tables.entry(mode.clone()).or_default(),
                    )?;
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    if explicit && section_rows == 0 {
        return Err("官方服务模式价格表为空".into());
    }
    let models = tables
        .remove("standard")
        .ok_or("官方页面缺少 Standard 价格，已保留上次价格")?;
    let mut card = RateCard {
        models,
        tiers: tables,
        date: date.into(),
    };
    for rate in card
        .models
        .values_mut()
        .chain(card.tiers.values_mut().flat_map(|v| v.values_mut()))
    {
        rate.verified_date = date.into();
    }
    if !card.validate() {
        return Err("官方价格表为空或无效".into());
    }
    Ok(card)
}
struct SyncState {
    snapshot: PricingSnapshot,
    completed: Option<Instant>,
    supplemental_cursor: usize,
}
pub struct PricingService {
    cache_path: PathBuf,
    inner: tokio::sync::Mutex<SyncState>,
}
impl PricingService {
    pub fn new(cache_path: PathBuf) -> Self {
        let mut snapshot = PricingSnapshot::bundled();
        if cache_path.exists() {
            match load_cache(&cache_path) {
                Ok(mut cached) => {
                    cached.status.using_cached_rates = true;
                    snapshot = cached;
                }
                Err(e) => {
                    snapshot.status.error = Some(e);
                    snapshot.status.status = "failed".into();
                }
            }
        }
        Self {
            cache_path,
            inner: tokio::sync::Mutex::new(SyncState {
                snapshot,
                completed: None,
                supplemental_cursor: 0,
            }),
        }
    }
    pub async fn refresh(&self, force: bool) -> PricingSnapshot {
        self.refresh_with(force, || async {
            fetch_markdown(SOURCE_MARKDOWN).await.and_then(|md| {
                parse_standard(&md, &chrono::Utc::now().format("%Y-%m-%d").to_string())
            })
        })
        .await
    }
    async fn refresh_with<F, Fut>(&self, force: bool, fetch: F) -> PricingSnapshot
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<RateCard, String>>,
    {
        let requested = Instant::now();
        let mut state = self.inner.lock().await;
        if state
            .completed
            .is_some_and(|last| last >= requested || (!force && last.elapsed() < DEBOUNCE))
        {
            return state.snapshot.clone();
        }
        state.snapshot.status.last_checked_at = Some(chrono::Utc::now().timestamp());
        let result = fetch().await;
        apply_refresh(&mut state.snapshot, result);
        // Refresh any previously verified models no longer listed on the central table.
        let supplemental: Vec<_> = state
            .snapshot
            .card
            .models
            .iter()
            .filter(|(id, r)| {
                r.source_url != SOURCE_URL
                    || state
                        .snapshot
                        .retained_rates
                        .contains_key(&format!("standard:{id}"))
            })
            .map(|(id, _)| id.clone())
            .collect();
        let supplemental = next_batch(supplemental, &mut state.supplemental_cursor);
        resolve_models(&mut state.snapshot, supplemental).await;
        if let Err(e) = persist_cache(&self.cache_path, &state.snapshot) {
            state.snapshot.status.status = "failed".into();
            state.snapshot.status.error = Some(e);
        }
        state.completed = Some(Instant::now());
        state.snapshot.clone()
    }
    pub async fn resolve_missing(&self, missing: &[MissingModel], force: bool) -> PricingSnapshot {
        let mut state = self.inner.lock().await;
        let now = chrono::Utc::now().timestamp();
        let mut models: Vec<_> = missing
            .iter()
            .map(|m| m.model.clone())
            .filter(|m| {
                valid_model(m)
                    && state.snapshot.card.lookup(m).is_none()
                    && (force
                        || state
                            .snapshot
                            .lookup_errors
                            .get(m)
                            .is_none_or(|e| now - e.checked_at >= 3600))
            })
            .collect();
        models.sort();
        models.dedup();
        models.truncate(4);
        if !models.is_empty() {
            resolve_models(&mut state.snapshot, models).await;
            if let Err(e) = persist_cache(&self.cache_path, &state.snapshot) {
                state.snapshot.status.status = "failed".into();
                state.snapshot.status.error = Some(e);
            }
        }
        state.snapshot.clone()
    }
}
fn next_batch(models: Vec<String>, cursor: &mut usize) -> Vec<String> {
    if models.is_empty() {
        *cursor = 0;
        return vec![];
    }
    let count = models.len().min(4);
    let result = (0..count)
        .map(|offset| models[(*cursor + offset) % models.len()].clone())
        .collect();
    *cursor = (*cursor + count) % models.len();
    result
}
async fn resolve_models(snapshot: &mut PricingSnapshot, models: Vec<String>) {
    let mut jobs = tokio::task::JoinSet::new();
    for model in models {
        jobs.spawn(async move {
            let result = fetch_markdown(&format!("{}.md", model_url(&model)))
                .await
                .and_then(|md| parse_model_page(&md, &model));
            (model, result)
        });
    }
    while let Some(Ok((model, result))) = jobs.join_next().await {
        apply_model_result(snapshot, model, result);
    }
    refresh_status(snapshot);
    snapshot.status.model_count = snapshot.card.models.len();
}
fn apply_model_result(
    snapshot: &mut PricingSnapshot,
    model: String,
    result: Result<ModelRates, String>,
) {
    match result {
        Ok(rate) => {
            snapshot.card.models.insert(model.clone(), rate);
            snapshot.lookup_errors.remove(&model);
            snapshot.retained_rates.remove(&format!("standard:{model}"));
        }
        Err(reason) => {
            snapshot.lookup_errors.insert(
                model,
                LookupError {
                    checked_at: chrono::Utc::now().timestamp(),
                    reason,
                },
            );
        }
    }
}
fn parse_model_page(markdown: &str, model: &str) -> Result<ModelRates, String> {
    if !markdown
        .lines()
        .any(|l| l.trim() == format!("Model ID: `{model}`"))
    {
        return Err("单模型官方页缺少精确模型 ID".into());
    }
    let section = markdown
        .split_once("\n## Pricing\n")
        .ok_or("单模型官方页缺少 Pricing 章节")?
        .1
        .split("\n## ")
        .next()
        .unwrap();
    // Do not infer context tiers or cache-write multipliers from free-form prose.
    let lower = section.to_ascii_lowercase();
    if lower.contains("long context")
        || lower.contains("long-context")
        || lower.contains("272")
        || lower.contains("cache write")
        || lower.contains("cache-write")
    {
        return Err("单模型页面包含额外上下文或缓存写入规则，需人工核验后才能估价".into());
    }
    let text = section
        .split_once("### Text tokens")
        .ok_or("缺少 Text tokens 定价表")?
        .1;
    let mut lines = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with('|'));
    if cells(lines.next().ok_or("缺少模型价格表")?) != vec!["Metric", "Price", "Unit"] {
        return Err("单模型价格表列发生变化".into());
    }
    lines.next().ok_or("缺少价格表分隔行")?;
    let mut rates = BTreeMap::new();
    for line in lines {
        if !line.trim_start().starts_with('|') {
            break;
        }
        let row = cells(line);
        if row.len() != 3
            || row[2] != "1M tokens"
            || !["Input", "Cached input", "Output"].contains(&row[0])
        {
            return Err("单模型价格表计价规则无法识别".into());
        }
        if rates.insert(row[0], price(row[1])?).is_some() {
            return Err("单模型价格表重复价格行".into());
        }
    }
    Ok(ModelRates {
        short: Rates {
            input: rates
                .get("Input")
                .copied()
                .flatten()
                .ok_or("缺少输入价格")?,
            cached: rates.get("Cached input").copied().flatten(),
            write: None,
            write_12h: None,
            output: rates
                .get("Output")
                .copied()
                .flatten()
                .ok_or("缺少输出价格")?,
        },
        long: None,
        threshold: None,
        source_url: model_url(model),
        verified_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
    })
}

fn apply_refresh(snapshot: &mut PricingSnapshot, result: Result<RateCard, String>) {
    match result {
        Ok(mut card) => {
            snapshot.retained_rates.clear();
            snapshot
                .lookup_errors
                .retain(|id, _| !card.models.contains_key(id));
            for (id, rate) in &snapshot.card.models {
                if !card.models.contains_key(id) {
                    card.models.insert(id.clone(), rate.clone());
                    snapshot
                        .retained_rates
                        .insert(format!("standard:{id}"), rate.verified_date.clone());
                }
            }
            for (tier, models) in &snapshot.card.tiers {
                for (id, rate) in models {
                    if !card.tiers.get(tier).is_some_and(|m| m.contains_key(id)) {
                        card.tiers
                            .entry(tier.clone())
                            .or_default()
                            .insert(id.clone(), rate.clone());
                        snapshot
                            .retained_rates
                            .insert(format!("{tier}:{id}"), rate.verified_date.clone());
                    }
                }
            }
            snapshot.card = card;
            snapshot.status.last_success_at = Some(chrono::Utc::now().timestamp());
            snapshot.main_error = None;
        }
        Err(error) => {
            snapshot.main_error = Some(error);
        }
    }
    refresh_status(snapshot);
    snapshot.status.model_count = snapshot.card.models.len();
}
fn refresh_status(snapshot: &mut PricingSnapshot) {
    let mut errors = Vec::new();
    if let Some(error) = &snapshot.main_error {
        errors.push(error.clone());
    }
    if !snapshot.retained_rates.is_empty() {
        errors.push(format!(
            "官方本次价格表未包含 {}；这些项保留此前核验价格",
            snapshot
                .retained_rates
                .iter()
                .map(|(id, date)| format!("{id}（{date}）"))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    for (id, error) in &snapshot.lookup_errors {
        errors.push(format!("{id} 补查失败：{}", error.reason));
    }
    snapshot.status.using_cached_rates =
        snapshot.main_error.is_some() || !snapshot.retained_rates.is_empty();
    snapshot.status.status = if errors.is_empty() {
        "upToDate"
    } else if snapshot.main_error.is_none() && snapshot.status.last_success_at.is_some() {
        "partial"
    } else {
        "failed"
    }
    .into();
    snapshot.status.error = (!errors.is_empty()).then(|| errors.join("；"));
}
fn load_cache(path: &Path) -> Result<PricingSnapshot, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_BYTES as u64 {
        return Err("价格缓存过大，已使用内置核验价格".into());
    }
    let bytes = fs::read(path).map_err(|e| format!("无法读取价格缓存：{e}"))?;
    if bytes.len() > MAX_BYTES {
        return Err("价格缓存过大，已使用内置核验价格".into());
    }
    let snapshot: PricingSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| "价格缓存损坏，已使用内置核验价格")?;
    if snapshot.version != 1
        || !snapshot.card.validate()
        || snapshot.status.source_url != SOURCE_URL
    {
        return Err("价格缓存版本或内容无效，已使用内置核验价格".into());
    }
    Ok(snapshot)
}
fn persist_cache(path: &Path, snapshot: &PricingSnapshot) -> Result<(), String> {
    let parent = path.parent().ok_or("价格缓存目录无效")?;
    fs::create_dir_all(parent).map_err(|e| format!("无法建立价格缓存目录：{e}"))?;
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let bytes = serde_json::to_vec(snapshot)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("无法保存价格缓存：{e}"))
}
async fn fetch_markdown(url: &str) -> Result<String, String> {
    // No shell, API key, conversation text or token history leaves the device.
    let mut child = tokio::process::Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "4",
            "--max-time",
            "10",
            "--max-filesize",
            "1048576",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("无法启动官方价格查询：{e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取价格查询结果")?;
    let stderr = child.stderr.take().ok_or("无法读取查询状态")?;
    let result = tokio::time::timeout(Duration::from_secs(12), async {
        let mut body = Vec::new();
        let mut error = Vec::new();
        let mut stdout = stdout.take((MAX_BYTES + 1) as u64);
        let mut stderr = stderr.take(4096);
        let (out, err) = tokio::join!(
            stdout.read_to_end(&mut body),
            stderr.read_to_end(&mut error)
        );
        out.map_err(|e| e.to_string())?;
        err.map_err(|e| e.to_string())?;
        if body.len() > MAX_BYTES {
            return Err("官方价格响应超过 1 MB 安全上限".into());
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "官方价格查询失败：{}",
                String::from_utf8_lossy(&error).trim()
            ));
        }
        String::from_utf8(body).map_err(|_| "官方价格响应不是 UTF-8 文本".into())
    })
    .await;
    result.unwrap_or_else(|_| Err("官方价格查询超时（12 秒），已保留上次价格".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn october_22_official_tables_parse_after_the_column_and_section_change() {
        let card = parse_standard(
            include_str!("fixtures/official-pricing-2026-10-22.md"),
            "2026-10-22",
        )
        .expect("current official text prices should parse");
        assert_eq!(card.models.len(), 43);
        assert_eq!(card.tiers["batch"].len(), 35);
        assert_eq!(card.tiers["flex"].len(), 20);
        assert_eq!(card.tiers["fast"].len(), 23);
        assert_eq!(card.tiers["ultrafast"].len(), 2);
        assert_eq!(card.models["gpt-6.1-sol"].short.input, 2.0);
        assert_eq!(card.models["gpt-6.1-sol"].short.output, 10.0);
        assert_eq!(card.models["gpt-6.1-sol"].short.write, Some(2.5));
        assert_eq!(card.models["gpt-6.1-sol"].short.write_12h, Some(5.0));
        assert_eq!(
            card.models["gpt-6.1-sol"].long.as_ref().unwrap().write_12h,
            Some(10.0)
        );
        assert_eq!(card.models["gpt-5.4-mini"].short.cached, Some(0.075));
        assert_eq!(card.models["gpt-5.6-cyber"].short.write, Some(15.625));
        for model in ["gpt-5.6-cyber", "gpt-5.5-cyber"] {
            assert_eq!(card.models[model].threshold, Some(272_000));
            assert!(card
                .estimate(
                    model,
                    Tokens {
                        input_tokens: 272_000,
                        ..Default::default()
                    }
                )
                .is_ok());
            assert_eq!(
                card.estimate(
                    model,
                    Tokens {
                        input_tokens: 272_001,
                        ..Default::default()
                    }
                )
                .unwrap_err(),
                "官方未公布此模型的长上下文价格"
            );
        }
        assert_eq!(card.models["gpt-rosalind-research"].short.output, 25.0);
        assert_eq!(card.models["gpt-5.3-codex"].short.output, 14.0);
        assert_eq!(card.tiers["batch"]["o3"].short.cached, None);
        assert_eq!(card.tiers["flex"]["o3"].short.cached, Some(0.25));
        assert!(!card.models.contains_key("gpt-image-1.5"));
        assert!(!card.models.contains_key("gpt-realtime-2.1"));
        assert!(!card.models.contains_key("gpt-4.1-2025-04-14"));
        assert_eq!(card.tiers.len(), 4);
    }
    #[test]
    fn cache_write_durations_are_separate_without_counting_tokens_twice() {
        let card = parse_standard(
            include_str!("fixtures/official-pricing-2026-10-22.md"),
            "2026-10-22",
        )
        .unwrap();
        let tokens = Tokens {
            input_tokens: 200_000,
            cached_input_tokens: 100_000,
            cache_write_input_tokens: 80_000,
            cache_write_12h_input_tokens: 30_000,
            output_tokens: 25_000,
        };
        // Astra: .20 ordinary + .10 read + .625 30m write + .75 12h write + 1.25 output.
        assert!(
            (card
                .estimate_tier("gpt-6-astra", tokens, Some("standard"))
                .unwrap()
                - 2.925)
                .abs()
                < 1e-9
        );
        assert!(
            (card
                .estimate_tier("gpt-6-astra", tokens, Some("fast"))
                .unwrap()
                - 5.85)
                .abs()
                < 1e-9
        );
        let long = Tokens {
            input_tokens: 300_000,
            ..tokens
        };
        assert!(
            (card
                .estimate_tier("gpt-6-astra", long, Some("standard"))
                .unwrap()
                - 7.225)
                .abs()
                < 1e-9
        );
        assert_eq!(tokens.total(), 225_000);
        let invalid = Tokens {
            cache_write_12h_input_tokens: 80_001,
            ..tokens
        };
        assert!(card
            .estimate("gpt-6-astra", invalid)
            .unwrap_err()
            .contains("12h"));
        // A legacy card has no verified 12h prices and must not use 30m instead.
        assert!(bundled_card()
            .estimate("gpt-6-astra", tokens)
            .unwrap_err()
            .contains("12h"));
        let old: Rates =
            serde_json::from_str(r#"{"input":10,"cached":1,"write":12.5,"output":50}"#).unwrap();
        assert_eq!(old.write_12h, None);
    }
    #[test]
    fn reordered_columns_and_identical_duplicates_are_safe_but_conflicts_fail() {
        let fixture = include_str!("fixtures/official-pricing-2026-10-22.md");
        let original = parse_standard(fixture, "today").unwrap();
        let reordered = fixture
            .lines()
            .map(|line| {
                if line.trim_start().starts_with('|') {
                    let mut row = cells(line);
                    row.reverse();
                    format!("| {} |", row.join(" | "))
                } else {
                    line.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = parse_standard(&reordered, "today").unwrap();
        assert_eq!(parsed.models, original.models);
        assert_eq!(parsed.tiers, original.tiers);
        // Cyber repeats the exact Standard Sol price. Altering only that copy
        // must fail instead of silently replacing the flagship price.
        let conflicting = fixture.replacen(
            "| gpt-5.6-sol | $4.00 | $20.00",
            "| gpt-5.6-sol | $99.00 | $20.00",
            1,
        );
        assert!(parse_standard(&conflicting, "today")
            .unwrap_err()
            .contains("冲突价格"));
        for changed in [
            fixture.replace("Short context 30m write", "Short context unknown write"),
            fixture.replace("| $2.50 | $5.00 | $4.00 |", "| $2.50 | $4.00 |"),
        ] {
            assert!(parse_standard(&changed, "today").is_err());
        }
    }
    #[test]
    fn latest_main_table_retains_absent_legacy_rates_as_partial_success() {
        let mut snapshot = PricingSnapshot::bundled();
        let latest = parse_standard(
            include_str!("fixtures/official-pricing-2026-10-22.md"),
            "2026-10-22",
        )
        .unwrap();
        apply_refresh(&mut snapshot, Ok(latest));
        assert_eq!(snapshot.status.status, "partial");
        assert!(snapshot.main_error.is_none());
        assert!(snapshot.status.last_success_at.is_some());
        assert_eq!(snapshot.retained_rates.len(), 7);
        assert_eq!(snapshot.card.models.len(), 47);
        assert_eq!(
            snapshot.card.models["gpt-6.1-sol"].verified_date,
            "2026-10-22"
        );
        assert_eq!(
            snapshot.card.models["gpt-3.5-turbo-1106"].verified_date,
            BUNDLED_DATE
        );
        let success = snapshot.status.last_success_at;
        apply_refresh(&mut snapshot, Err("invalid official price table".into()));
        assert_eq!(snapshot.status.status, "failed");
        assert_eq!(snapshot.status.last_success_at, success);
        assert_eq!(
            snapshot.card.models["gpt-6.1-sol"].short.write_12h,
            Some(5.0)
        );
    }
    #[test]
    fn legacy_lookup_failure_does_not_hide_a_successful_main_table_refresh() {
        let mut snapshot = PricingSnapshot::bundled();
        apply_model_result(
            &mut snapshot,
            "gpt-3.5-turbo-1106".into(),
            Err("HTTP 404".into()),
        );
        refresh_status(&mut snapshot);
        assert_eq!(snapshot.status.status, "failed"); // No successful main fetch yet.
        apply_refresh(
            &mut snapshot,
            parse_standard(
                include_str!("fixtures/official-pricing-2026-10-22.md"),
                "2026-10-22",
            ),
        );
        assert_eq!(snapshot.status.status, "partial");
        assert!(snapshot.status.error.as_ref().unwrap().contains("HTTP 404"));
        assert!(snapshot
            .status
            .error
            .as_ref()
            .unwrap()
            .contains("2026-10-13"));
        assert!(snapshot.status.last_success_at.is_some());
        assert!(snapshot.card.models.contains_key("gpt-5.6-cyber"));
        assert!(snapshot.card.models.contains_key("gpt-3.5-turbo-1106"));
    }
    #[test]
    fn current_gpt_6_1_sol_tokens_have_a_verified_price() {
        assert_eq!(
            estimate(
                "gpt-6.1-sol",
                Tokens {
                    input_tokens: 1_000,
                    cached_input_tokens: 800,
                    output_tokens: 100,
                    ..Default::default()
                }
            ),
            Some(0.00148)
        );
    }
    #[test]
    fn cached_input_is_not_counted_twice() {
        assert!(
            (estimate(
                "gpt-5.6-sol",
                Tokens {
                    input_tokens: 100_000,
                    cached_input_tokens: 80_000,
                    output_tokens: 10_000,
                    ..Default::default()
                }
            )
            .unwrap()
                - 0.312)
                .abs()
                < 1e-9
        );
    }
    #[test]
    fn astra_fast_api_cost_does_not_apply_subscription_or_credit_multipliers() {
        let tokens = Tokens {
            input_tokens: 200_000,
            cached_input_tokens: 100_000,
            cache_write_input_tokens: 80_000,
            output_tokens: 25_000,
            ..Default::default()
        };
        let card = bundled_card();
        // Standard: .20 ordinary + .10 cache read + 1.00 cache write + 1.25 output.
        let standard = card
            .estimate_tier("gpt-6-astra", tokens, Some("default"))
            .unwrap();
        // Fast has its own 20 / 2 / 25 / 100 rates. The 2.5x subscription rule
        // and 2x credit rule must never be reapplied to this API amount.
        let fast = card
            .estimate_tier("gpt-6-astra", tokens, Some("fast"))
            .unwrap();
        assert!((standard - 2.55).abs() < 1e-9);
        assert!((fast - 5.10).abs() < 1e-9);
        assert!(
            (card
                .estimate_tier("gpt-6-astra", tokens, Some("priority"))
                .unwrap()
                - fast)
                .abs()
                < 1e-9
        );
        assert_eq!(tokens.total(), 225_000);
    }
    #[test]
    fn long_context_and_writes_are_priced_per_request() {
        let t = Tokens {
            input_tokens: 300_000,
            cached_input_tokens: 200_000,
            cache_write_input_tokens: 20_000,
            output_tokens: 10_000,
            ..Default::default()
        };
        assert!((estimate("gpt-5.6-sol", t).unwrap() - 1.3).abs() < 1e-9);
    }
    #[test]
    fn unknown_models_are_not_free_or_guessed() {
        for m in ["gpt-5.6-sol-experiment", "codex-auto-review", "gpt-5.6"] {
            assert_eq!(estimate(m, Tokens::default()), None);
        }
        assert_eq!(
            estimate(
                "gpt-5.5",
                Tokens {
                    input_tokens: 1,
                    cached_input_tokens: 2,
                    ..Default::default()
                }
            ),
            None
        );
        assert_eq!(
            estimate(
                "gpt-5.5",
                Tokens {
                    input_tokens: 1,
                    cache_write_input_tokens: 1,
                    ..Default::default()
                }
            ),
            None
        );
    }
    #[test]
    fn exact_snapshot_prices_are_used_and_unverified_dated_aliases_are_not_guessed() {
        assert_eq!(estimate("gpt-5.5-2026-04-23", Tokens::default()), None);
        assert_eq!(
            estimate(
                "gpt-4o-2024-05-13",
                Tokens {
                    input_tokens: 100_000,
                    ..Default::default()
                }
            ),
            Some(0.5)
        );
        assert_eq!(
            estimate(
                "gpt-4o-2024-08-06",
                Tokens {
                    input_tokens: 100_000,
                    ..Default::default()
                }
            ),
            None
        );
        assert_eq!(estimate("中文中文中文中文", Tokens::default()), None);
    }
    #[test]
    fn parses_only_standard_and_rejects_incomplete_or_changed_tables() {
        let fixture = include_str!("fixtures/official-pricing-2026-10-13.md");
        let md = fixture.replace(
            "| gpt-6.1-sol | $1.00 | $0.05",
            "| gpt-6.1-sol | $0.01 | $0.05",
        );
        assert_eq!(
            parse_standard(&md, "today").unwrap().models["gpt-6.1-sol"]
                .short
                .input,
            2.0
        );
        for md in [
            format!("{fixture}\n### Batch pricing data\n| gpt-6.1-sol | $0.01 |\n"),
            fixture.replace("Short context cache writes", "Cache writes"),
            fixture.replace("$0.10", "NaN"),
            fixture.replace("$2.00 | $0.10 | $2.50 | $10.00", "$2.00 | $0.10 | $2.50"),
            fixture.replace("≤272K", "≤300K"),
        ] {
            assert!(parse_standard(&md, "today").is_err());
        }
        let changed = fixture.replace("$0.10 | $2.50 | $10.00", "$0.08 | $2.75 | $11.00");
        let card = parse_standard(&changed, "today").unwrap();
        assert_eq!(card.models["gpt-6.1-sol"].short.cached, Some(0.08));
        assert_eq!(card.models["gpt-6.1-sol"].short.write, Some(2.75));
    }
    #[test]
    fn failed_refresh_and_atomic_cache_preserve_last_good_rates() {
        let mut s = PricingSnapshot::bundled();
        apply_refresh(&mut s, Ok(bundled_card().clone()));
        let success = s.status.last_success_at;
        apply_refresh(&mut s, Err("network offline".into()));
        assert_eq!(s.status.last_success_at, success);
        assert!(s.status.using_cached_rates);
        assert_eq!(s.status.status, "failed");
        assert_eq!(s.card.models["gpt-6.1-sol"].short.cached, Some(0.1));
        let dir = std::env::temp_dir().join(format!("codex-pricing-test-{}", std::process::id()));
        let path = dir.join("prices.json");
        persist_cache(&path, &s).unwrap();
        let recovered = load_cache(&path).unwrap();
        assert_eq!(recovered.status.error.as_deref(), Some("network offline"));
        assert_eq!(recovered.card.models.len(), s.card.models.len());
        fs::write(&path, "broken").unwrap();
        assert!(load_cache(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn source_links_are_constrained_to_official_pricing_or_exact_model_pages() {
        for url in [
            SOURCE_URL,
            "https://developers.openai.com/api/docs/models/gpt-6.1-sol",
        ] {
            assert!(is_pricing_url(url));
        }
        for url in [
            "https://developers.openai.com.evil/api/docs/pricing",
            "https://developers.openai.com/api/docs/models/../pricing",
            "http://developers.openai.com/api/docs/pricing",
            "https://developers.openai.com/api/docs/models/gpt-6?x=1",
        ] {
            assert!(!is_pricing_url(url));
        }
    }
    #[test]
    fn fast_rates_are_independent_and_tokens_are_not_multiplied() {
        let card = bundled_card();
        let t = Tokens {
            input_tokens: 100_000,
            cached_input_tokens: 80_000,
            output_tokens: 10_000,
            ..Default::default()
        };
        let standard = card
            .estimate_tier("gpt-6.1-sol", t, Some("default"))
            .unwrap();
        let fast = card
            .estimate_tier("gpt-6.1-sol", t, Some("priority"))
            .unwrap();
        assert!((standard - 0.148).abs() < 1e-9);
        assert!((fast - 0.296).abs() < 1e-9);
        assert_eq!(t.total(), 110_000);
        assert_eq!(
            card.estimate_tier("gpt-6.1-sol", t, None).unwrap(),
            standard
        );
        assert!(card
            .estimate_tier("gpt-6.1-sol", t, Some("new-mode"))
            .is_err());
        assert!(
            (card
                .estimate_tier("gpt-6.1-sol", t, Some("ultrafast"))
                .unwrap()
                - 0.888)
                .abs()
                < 1e-9
        );
    }
    #[test]
    fn missing_long_context_price_is_not_replaced_by_short_context_price() {
        let card = bundled_card();
        let at = Tokens {
            input_tokens: 272_000,
            ..Default::default()
        };
        let above = Tokens {
            input_tokens: 272_001,
            ..Default::default()
        };
        for (model, tier) in [
            ("gpt-5.5", "fast"),
            ("gpt-5.4", "fast"),
            ("gpt-5.5-pro", "batch"),
            ("gpt-5.5-pro", "flex"),
        ] {
            assert!(card.estimate_tier(model, at, Some(tier)).is_ok());
            assert_eq!(
                card.estimate_tier(model, above, Some(tier)).unwrap_err(),
                "官方未公布此模型的长上下文价格"
            );
        }
        // Unqualified rows have one published rate; do not invent a tier boundary.
        assert!(card
            .estimate_tier("gpt-5.4-mini", above, Some("standard"))
            .is_ok());
    }
    #[test]
    fn discovers_new_model_and_updated_fast_prices_from_the_next_official_table() {
        let fixture = include_str!("fixtures/official-pricing-2026-10-13.md");
        let next = fixture.replace("gpt-6.1-sol", "gpt-7-sol").replace(
            "$4.00 | $0.20 | $5.00 | $20.00",
            "$6.00 | $0.30 | $7.50 | $30.00",
        );
        let card = parse_standard(&next, "2030-01-01").unwrap();
        let future_mode = next.replace("### Ultrafast pricing data", "### Hyperfast pricing data");
        let future_card = parse_standard(&future_mode, "2030-01-01").unwrap();
        assert!(future_card
            .estimate_tier("gpt-7-sol", Tokens::default(), Some("hyperfast"))
            .is_ok());
        assert!(card.models.contains_key("gpt-7-sol"));
        assert_eq!(card.tiers["fast"]["gpt-7-sol"].short.input, 6.0);
        let mut snapshot = PricingSnapshot::bundled();
        apply_refresh(&mut snapshot, Ok(card));
        assert!(snapshot.card.models.contains_key("gpt-6.1-sol"));
        assert!(snapshot.status.using_cached_rates);
        assert!(snapshot
            .status
            .error
            .as_ref()
            .unwrap()
            .contains("2026-10-13"));
    }
    #[test]
    fn supplemental_lookup_clears_old_failure_only_after_verified_success() {
        let page="Model ID: `gpt-new`\n\n## Pricing\n\n### Text tokens\n\n| Metric | Price | Unit |\n| --- | --- | --- |\n| Input | $2 | 1M tokens |\n| Cached input | $0.1 | 1M tokens |\n| Output | $10 | 1M tokens |\n";
        let mut snapshot = PricingSnapshot::bundled();
        apply_refresh(&mut snapshot, Ok(bundled_card().clone()));
        apply_model_result(&mut snapshot, "gpt-new".into(), Err("404".into()));
        refresh_status(&mut snapshot);
        assert_eq!(snapshot.status.status, "partial");
        apply_model_result(
            &mut snapshot,
            "gpt-new".into(),
            parse_model_page(page, "gpt-new"),
        );
        refresh_status(&mut snapshot);
        assert_eq!(snapshot.status.status, "upToDate");
        assert!(snapshot.status.error.is_none());
        assert!(snapshot.card.lookup("gpt-new").is_some());
        assert!(parse_model_page(page, "gpt-other").is_err());
        assert!(parse_model_page(&page.replace("1M tokens", "1K tokens"), "gpt-new").is_err());
        assert!(
            parse_model_page(&format!("{page}\nLong context changes prices."), "gpt-new").is_err()
        );
    }
    #[tokio::test]
    async fn concurrent_refreshes_share_a_single_fetch_and_force_can_retry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let dir =
            std::env::temp_dir().join(format!("codex-pricing-concurrent-{}", std::process::id()));
        let service = PricingService::new(dir.join("prices.json"));
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(bundled_card().clone())
        };
        let (a, b) = tokio::join!(
            service.refresh_with(true, fetch),
            service.refresh_with(true, fetch)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(a.card.models.len(), b.card.models.len());
        service.refresh_with(false, fetch).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let failed = service
            .refresh_with(true, || async { Err("offline".into()) })
            .await;
        assert_eq!(failed.status.status, "failed");
        assert!(failed.card.models.contains_key("gpt-6.1-sol"));
        let success = service.refresh_with(true, fetch).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(success.status.status, "upToDate");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn supplemental_lookup_rotates_past_the_first_four_models() {
        let candidates: Vec<_> = (0..9).map(|n| format!("model-{n}")).collect();
        let mut cursor = 0;
        let mut checked = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let batch = next_batch(candidates.clone(), &mut cursor);
            assert_eq!(batch.len(), 4);
            checked.extend(batch);
        }
        assert_eq!(checked.len(), 9);
        assert!(next_batch(vec![], &mut cursor).is_empty());
        assert_eq!(cursor, 0);
    }
    #[tokio::test]
    #[ignore = "live official pricing request; writes temporary cache only"]
    async fn live_official_price_sync() {
        let dir = std::env::temp_dir().join(format!("codex-pricing-live-{}", std::process::id()));
        let service = PricingService::new(dir.join("prices.json"));
        let s = service.refresh(true).await;
        assert!(s.main_error.is_none(), "{:?}", s.main_error);
        assert!(s.status.last_success_at.is_some(), "{:?}", s.status.error);
        assert!(
            ["upToDate", "partial"].contains(&s.status.status.as_str()),
            "{:?}",
            s.status.error
        );
        // A removed legacy model page may return 404 while the current central
        // table succeeds. Keep that failure visible, with its last good price.
        assert!(s
            .lookup_errors
            .keys()
            .all(|model| s.card.models.contains_key(model)));
        assert!(s.card.models.contains_key("gpt-6.1-sol"));
        assert_eq!(s.card.models["gpt-6.1-sol"].verified_date, s.card.date);
        assert!(s.card.models["gpt-6.1-sol"].short.write_12h.is_some());
        eprintln!(
            "official_models={} checked={:?} gpt6.1_cached={:?} tier_counts={:?} retained={} status={}",
            s.card.models.len(),
            s.status.last_checked_at,
            s.card.models["gpt-6.1-sol"].short.cached,
            s.card.tiers.iter().map(|(tier, models)| (tier, models.len())).collect::<Vec<_>>(),
            s.retained_rates.len(),
            s.status.status
        );
        fs::write(
            "/tmp/codex-usage-verified-pricing.json",
            serde_json::to_vec_pretty(&s).unwrap(),
        )
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
