mod executions;
mod modes;
mod parser;
pub mod pricing;
mod projects;
mod trend;
pub mod types;

use parser::ParsedFile;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};
use types::*;

#[derive(Default)]
struct CachedFile {
    parsed: ParsedFile,
    length: u64,
    modified: Option<SystemTime>,
}

#[derive(Default)]
pub struct AnalyticsStore {
    files: HashMap<PathBuf, CachedFile>,
    pub pricing: pricing::PricingSnapshot,
    mode_evidence: modes::ModeEvidence,
}

#[derive(Default)]
pub struct AnalyticsState {
    pub store: Mutex<AnalyticsStore>,
    pub current_window: Mutex<Option<ResetWindow>>,
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>, unreadable: &mut usize) {
    if !path.exists() {
        return;
    }
    let entries = match fs::read_dir(path) {
        Ok(v) => v,
        Err(_) => {
            *unreadable += 1;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *unreadable += 1;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *unreadable += 1;
            continue;
        };
        // Never follow symlinks out of the explicitly scoped Codex history directories.
        if kind.is_dir() {
            collect_files(&entry.path(), out, unreadable)
        } else if kind.is_file() && entry.path().extension().is_some_and(|s| s == "jsonl") {
            out.push(entry.path());
        }
    }
}

pub fn codex_home() -> Result<PathBuf, String> {
    if let Some(directories) = crate::runtime_paths::isolated_directories()? {
        return Ok(directories.history.clone());
    }
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))
        .ok_or_else(|| "无法定位本机 Codex 历史目录".into())
}

fn thread_titles(home: &Path) -> HashMap<String, String> {
    use std::io::{BufRead, BufReader};
    let mut titles = HashMap::new();
    if let Ok(file) = fs::File::open(home.join("session_index.jsonl")) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if line.len() > 32_768 {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if let (Some(id), Some(title)) = (v["id"].as_str(), v["thread_name"].as_str()) {
                    let title: String =
                        title.chars().filter(|c| !c.is_control()).take(90).collect();
                    if !title.trim().is_empty() {
                        titles.insert(id.to_string(), title);
                    }
                }
            }
        }
    }
    titles
}

fn conversation_title(thread: &str, titles: &HashMap<String, String>) -> String {
    titles
        .get(thread)
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| format!("对话 {}", thread.chars().take(8).collect::<String>()))
}

impl AnalyticsStore {
    pub fn configure_mode_cache(&mut self, path: PathBuf) {
        self.mode_evidence.configure_cache(path);
    }
    pub fn refresh_modes(&mut self, home: &Path) {
        self.mode_evidence.refresh(home);
    }
    pub fn query(
        &mut self,
        home: &Path,
        query: Query,
        now: i64,
        current: Option<ResetWindow>,
    ) -> Result<Dashboard, String> {
        if ![1, 12, 24, 168, 720].contains(&query.hours) {
            return Err("不支持的统计时间范围".into());
        }
        let mut coverage = Coverage::default();
        let mut paths = Vec::new();
        collect_files(
            &home.join("sessions"),
            &mut paths,
            &mut coverage.unreadable_files,
        );
        collect_files(
            &home.join("archived_sessions"),
            &mut paths,
            &mut coverage.unreadable_files,
        );
        paths.sort();
        let existing: HashSet<_> = paths.iter().cloned().collect();
        self.files.retain(|p, _| existing.contains(p));
        for path in paths {
            let Ok(meta) = fs::metadata(&path) else {
                coverage.unreadable_files += 1;
                continue;
            };
            let cached = self.files.entry(path.clone()).or_default();
            let modified = meta.modified().ok();
            if meta.len() != cached.length || modified != cached.modified {
                if meta.len() < cached.length
                    || (meta.len() == cached.length && modified != cached.modified)
                {
                    cached.parsed = ParsedFile::default();
                }
                if cached.parsed.read_append(&path).is_err() {
                    coverage.unreadable_files += 1;
                    continue;
                }
                cached.length = meta.len();
                cached.modified = modified;
            }
            coverage.files += 1;
            coverage.malformed_lines += cached.parsed.malformed;
            coverage.oversized_lines += cached.parsed.oversized;
        }
        let modern: HashSet<_> = self
            .files
            .values()
            .flat_map(|f| f.parsed.effective_events())
            .filter(|e| e.modern)
            .map(|e| e.turn.clone())
            .collect();
        let mut dedup = BTreeMap::new();
        let mut turns: HashMap<String, Turn> = HashMap::new();
        let mut windows = Vec::new();
        // Sorting the paths makes the older original win when a fork contains inherited events.
        let mut files: Vec<_> = self.files.iter().collect();
        files.sort_by_key(|(p, _)| *p);
        for (_, file) in files {
            for e in file.parsed.effective_events() {
                if !e.modern && modern.contains(&e.turn) {
                    continue;
                }
                if e.model == "codex-auto-review" || e.model == "codex-internal" {
                    coverage.excluded_internal_events += 1;
                    continue;
                }
                if e.at > now {
                    continue;
                }
                dedup.entry(e.id.clone()).or_insert_with(|| e.clone());
            }
            for (id, turn) in &file.parsed.turns {
                let t = turns.entry(id.clone()).or_insert_with(|| turn.clone());
                if t.completed.is_none() {
                    t.completed = turn.completed;
                }
                if t.title.is_empty() {
                    t.title = turn.title.clone();
                }
            }
            windows.extend(file.parsed.windows.iter().cloned());
        }
        if let Some(w) = current {
            windows.push(w);
        }
        let titles = thread_titles(home);
        let mut events: Vec<UsageEvent> = dedup.into_values().collect();
        self.mode_evidence.refresh(home);
        for e in &mut events {
            self.mode_evidence.recover(e);
            // Modern records carry a root execution ID; older child records can only be
            // attributed when their direct parent's execution interval is unambiguous.
            if e.root_turn.is_empty() {
                if !e.is_subagent {
                    e.root_turn = e.turn.clone();
                } else if let Some(parent) = &e.parent {
                    let matches: Vec<_> = turns
                        .values()
                        .filter(|t| {
                            &t.thread == parent
                                && t.started <= e.at
                                && t.completed.unwrap_or(now) >= e.at
                        })
                        .collect();
                    if matches.len() == 1 {
                        e.root_turn = if matches[0].root_turn.is_empty() {
                            matches[0].id.clone()
                        } else {
                            matches[0].root_turn.clone()
                        };
                    }
                }
            }
            if let Some(root) = turns.get(&e.root_turn) {
                e.project = root.project.clone();
            }
            if e.project.is_empty() {
                e.project = "未归属项目".into();
            }
            if e.is_subagent && (e.root_turn.is_empty() || !turns.contains_key(&e.root_turn)) {
                coverage.unassigned_events += 1;
            }
        }
        events.sort_by_key(|e| e.at);
        coverage.first_event_at = events.first().map(|e| e.at);
        coverage.last_event_at = events.last().map(|e| e.at);
        Ok(build_dashboard_with_prices(
            &events,
            &turns,
            &titles,
            windows,
            coverage,
            query,
            now,
            &projects::ProjectCatalog::read(home),
            &self.pricing,
        ))
    }
}

#[cfg(test)]
fn metrics<'a>(events: impl Iterator<Item = &'a UsageEvent>) -> Metrics {
    metrics_with_card(events, &pricing::PricingSnapshot::bundled().card)
}

fn metrics_with_card<'a>(
    events: impl Iterator<Item = &'a UsageEvent>,
    card: &pricing::RateCard,
) -> Metrics {
    let mut m = Metrics::default();
    let mut main = HashSet::new();
    let mut sub = HashSet::new();
    let mut tiers: BTreeMap<String, ServiceTierMetrics> = BTreeMap::new();
    for e in events {
        m.input_tokens = m.input_tokens.saturating_add(e.tokens.input_tokens);
        m.cached_input_tokens = m
            .cached_input_tokens
            .saturating_add(e.tokens.cached_input_tokens);
        m.cache_write_input_tokens = m
            .cache_write_input_tokens
            .saturating_add(e.tokens.cache_write_input_tokens);
        m.output_tokens = m.output_tokens.saturating_add(e.tokens.output_tokens);
        m.total_tokens = m.total_tokens.saturating_add(e.tokens.total());
        let recorded_tier = pricing::normalize_tier(e.service_tier.as_deref());
        // User policy: missing historical modes count as Standard. Keep the
        // assumption in metrics, never write it into the original event/evidence.
        let assumed_standard = recorded_tier == "unknown";
        let tier_name = if assumed_standard {
            "standard".to_string()
        } else {
            recorded_tier
        };
        let tier = tiers
            .entry(tier_name.clone())
            .or_insert_with(|| ServiceTierMetrics {
                tier: tier_name,
                ..Default::default()
            });
        tier.tokens = tier.tokens.saturating_add(e.tokens.total());
        if assumed_standard {
            m.assumed_standard_tokens = m.assumed_standard_tokens.saturating_add(e.tokens.total());
            tier.assumed_standard_tokens = tier
                .assumed_standard_tokens
                .saturating_add(e.tokens.total());
        }
        if e.request_mode {
            m.request_mode_tokens = m.request_mode_tokens.saturating_add(e.tokens.total());
            tier.request_mode_tokens = tier.request_mode_tokens.saturating_add(e.tokens.total());
        }
        if let Ok(cost) = card.estimate_tier(&e.model, e.tokens, e.service_tier.as_deref()) {
            m.cost_usd += cost;
            tier.cost_usd += cost;
        } else {
            m.unpriced_tokens = m.unpriced_tokens.saturating_add(e.tokens.total());
            tier.unpriced_tokens = tier.unpriced_tokens.saturating_add(e.tokens.total());
        }
        if e.is_subagent {
            sub.insert(&e.thread);
        } else {
            main.insert(&e.thread);
        }
    }
    m.service_tiers = tiers.into_values().collect();
    m.main_agent_count = main.len();
    m.sub_agent_count = sub.len();
    m
}

fn completed_root_ids(turns: &HashMap<String, Turn>, start: i64, end: i64) -> HashSet<&str> {
    turns
        .values()
        .filter_map(|turn| {
            let completed = turn.completed?;
            (!turn.is_subagent && completed >= start && completed < end).then_some(turn.id.as_str())
        })
        .collect()
}

fn events_for_completed_tasks<'a>(
    events: &'a [UsageEvent],
    turns: &HashMap<String, Turn>,
    start: i64,
    end: i64,
) -> Vec<&'a UsageEvent> {
    let completed_roots = completed_root_ids(turns, start, end);
    events
        .iter()
        .filter(|event| completed_roots.contains(event.root_turn.as_str()))
        .collect()
}

fn events_for_cycle<'a>(
    events: &'a [UsageEvent],
    turns: &HashMap<String, Turn>,
    start: i64,
    end: i64,
) -> Vec<&'a UsageEvent> {
    let completed_roots = completed_root_ids(turns, start, end);
    events
        .iter()
        .filter(|event| {
            completed_roots.contains(event.root_turn.as_str())
                || (event.at >= start
                    && event.at < end
                    && turns
                        .get(&event.root_turn)
                        .is_none_or(|turn| turn.is_subagent || turn.thread.is_empty()))
        })
        .collect()
}

fn completed_task_metrics(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    start: i64,
    end: i64,
    card: &pricing::RateCard,
) -> Metrics {
    metrics_with_card(
        events_for_completed_tasks(events, turns, start, end).into_iter(),
        card,
    )
}

fn share(id: String, name: String, kind: ShareKind, tokens: u64, total: u64) -> Share {
    Share {
        id,
        name,
        kind,
        tokens,
        percent: if total == 0 {
            0.0
        } else {
            tokens as f64 / total as f64 * 100.0
        },
    }
}

fn model_shares(events: &[&UsageEvent]) -> Vec<Share> {
    let mut grouped: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    for e in events {
        let name = if e.model.is_empty() {
            "未知模型"
        } else {
            &e.model
        };
        *grouped.entry(name.into()).or_default() += e.tokens.total();
        total += e.tokens.total();
    }
    let mut result: Vec<_> = grouped
        .into_iter()
        .map(|(name, tokens)| share(name.clone(), name, ShareKind::Model, tokens, total))
        .collect();
    result.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.id.cmp(&b.id)));
    result
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum AttributionKey {
    Project(String),
    Conversation(String),
    Unassigned,
}

fn attribution_key(
    event: &UsageEvent,
    turns: &HashMap<String, Turn>,
    catalog: &projects::ProjectCatalog,
) -> AttributionKey {
    let root = turns
        .get(&event.root_turn)
        .filter(|turn| !turn.is_subagent && !turn.thread.is_empty());
    let (thread, cwd) = if let Some(root) = root {
        (root.thread.as_str(), root.project.as_str())
    } else if !event.is_subagent && !event.thread.is_empty() {
        (event.thread.as_str(), event.project.as_str())
    } else {
        return AttributionKey::Unassigned;
    };
    catalog
        .resolve(thread, cwd)
        .map(|project| AttributionKey::Project(project.id))
        .unwrap_or_else(|| AttributionKey::Conversation(thread.to_string()))
}

fn attribution_shares(
    events: &[&UsageEvent],
    turns: &HashMap<String, Turn>,
    titles: &HashMap<String, String>,
    catalog: &projects::ProjectCatalog,
) -> Vec<Share> {
    let mut grouped: BTreeMap<AttributionKey, u64> = BTreeMap::new();
    let mut total = 0u64;
    for event in events {
        *grouped
            .entry(attribution_key(event, turns, catalog))
            .or_default() += event.tokens.total();
        total += event.tokens.total();
    }
    let mut result: Vec<_> = grouped
        .into_iter()
        .map(|(key, tokens)| match key {
            AttributionKey::Project(id) => share(
                id.clone(),
                catalog.name(&id),
                ShareKind::Project,
                tokens,
                total,
            ),
            AttributionKey::Conversation(id) => share(
                id.clone(),
                conversation_title(&id, titles),
                ShareKind::Conversation,
                tokens,
                total,
            ),
            AttributionKey::Unassigned => share(
                "unassigned".into(),
                "无法归属".into(),
                ShareKind::Unassigned,
                tokens,
                total,
            ),
        })
        .collect();
    result.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });
    result
}

fn cycles(windows: Vec<ResetWindow>, now: i64) -> Vec<Cycle> {
    let mut starts = BTreeMap::new();
    for w in windows {
        if w.seconds > 0 && w.end - w.seconds <= now {
            starts.insert(w.end - w.seconds, w.end);
        }
    }
    let entries: Vec<_> = starts.into_iter().collect();
    let mut result = Vec::new();
    for (i, (start, end)) in entries.iter().enumerate() {
        let end = entries
            .get(i + 1)
            .map(|(next, _)| (*end).min(*next))
            .unwrap_or(*end);
        if end <= *start {
            continue;
        }
        result.push(Cycle {
            id: start.to_string(),
            start: *start,
            end,
            ongoing: *start <= now && end > now,
            estimated: true,
        });
    }
    result.reverse();
    result.truncate(24);
    result
}

fn build_dashboard_with_prices(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    titles: &HashMap<String, String>,
    windows: Vec<ResetWindow>,
    coverage: Coverage,
    query: Query,
    now: i64,
    catalog: &projects::ProjectCatalog,
    snapshot: &pricing::PricingSnapshot,
) -> Dashboard {
    let start = now - i64::from(query.hours) * 3600;
    let period_seconds = i64::from(query.hours) * 3600;
    let period_end = now.saturating_add(1);
    let history = completed_task_metrics(events, turns, start, period_end, &snapshot.card);
    let previous_start = start - period_seconds;
    let comparisons = UsageComparisons {
        previous: completed_task_metrics(events, turns, previous_start, start, &snapshot.card),
    };
    let (trend, trend_bucket_seconds, trend_has_older) = trend::buckets_by_completion(
        events,
        turns,
        query.hours,
        query.trend_before,
        now,
        &snapshot.card,
    );
    let billing_start = query
        .billing_start
        .filter(|v| *v <= now && *v >= now - 32 * 86400);
    let billing = completed_task_metrics(
        events,
        turns,
        billing_start.unwrap_or(now - 30 * 86400),
        period_end,
        &snapshot.card,
    );
    let cycles = cycles(windows, now);
    let selected = query
        .cycle_id
        .as_ref()
        .and_then(|id| cycles.iter().find(|c| &c.id == id))
        .or_else(|| cycles.iter().find(|c| c.ongoing))
        .or(cycles.first());
    let cycle = selected.map(|c| {
        let filtered = events_for_cycle(events, turns, c.start, c.end.min(period_end));
        CycleDetails {
            cycle: c.clone(),
            metrics: metrics_with_card(filtered.iter().copied(), &snapshot.card),
            models: model_shares(&filtered),
            attributions: attribution_shares(&filtered, turns, titles, catalog),
        }
    });
    let execution_results = executions::query_executions_with_prices(
        events,
        turns,
        titles,
        catalog,
        &query,
        now,
        &snapshot.card,
    );
    Dashboard {
        updated_at: now,
        start,
        end: now,
        history,
        comparisons,
        trend,
        trend_bucket_seconds,
        trend_has_older,
        billing_start: billing_start.unwrap_or(now - 30 * 86400),
        billing_is_rolling: billing_start.is_none(),
        billing,
        cycles,
        cycle,
        executions: execution_results.rows,
        projects: execution_results.projects,
        conversations: execution_results.conversations,
        execution_page: execution_results.page,
        sessions: execution_results.sessions,
        session_page: execution_results.session_page,
        coverage,
        pricing_date: snapshot.card.date.clone(),
        pricing: snapshot.status_for(events),
    }
}

#[cfg(test)]
fn build_dashboard(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    titles: &HashMap<String, String>,
    windows: Vec<ResetWindow>,
    coverage: Coverage,
    query: Query,
    now: i64,
    catalog: &projects::ProjectCatalog,
) -> Dashboard {
    build_dashboard_with_prices(
        events,
        turns,
        titles,
        windows,
        coverage,
        query,
        now,
        catalog,
        &pricing::PricingSnapshot::bundled(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(id: &str, at: i64, sub: bool) -> UsageEvent {
        UsageEvent {
            id: id.into(),
            at,
            thread: if sub { "child" } else { "main" }.into(),
            turn: "t".into(),
            root_turn: "t".into(),
            parent: None,
            project: "Usage".into(),
            model: "gpt-5.6-sol".into(),
            service_tier: None,
            request_mode: false,
            is_subagent: sub,
            tokens: Tokens {
                input_tokens: 100,
                cached_input_tokens: 50,
                output_tokens: 10,
                ..Default::default()
            },
            modern: true,
        }
    }

    fn attribution_event(
        id: &str,
        at: i64,
        thread: &str,
        root_turn: &str,
        cwd: &str,
        subagent: bool,
        tokens: u64,
    ) -> UsageEvent {
        UsageEvent {
            id: id.into(),
            at,
            thread: thread.into(),
            turn: id.into(),
            root_turn: root_turn.into(),
            parent: None,
            project: cwd.into(),
            model: "gpt-5.6-sol".into(),
            service_tier: None,
            request_mode: false,
            is_subagent: subagent,
            tokens: Tokens {
                input_tokens: tokens,
                ..Default::default()
            },
            modern: true,
        }
    }

    fn attribution_fixture() -> (
        Vec<UsageEvent>,
        HashMap<String, Turn>,
        projects::ProjectCatalog,
        i64,
    ) {
        let now = 1_800_000_000;
        let project_id = "d02002a2-80d0-481c-89b9-c972fb22d4ab";
        let catalog = serde_json::from_str(&format!(
            r#"{{"projectless-thread-ids":["solo-1","solo-2"],"thread-project-assignments":{{"project-a":{{"projectId":"{project_id}"}},"project-b":{{"projectId":"{project_id}"}}}},"local-projects":{{"{project_id}":{{"name":"Codex Usage","rootPaths":["/work/Codex Usage"]}}}}}}"#
        ))
        .unwrap();
        let mut turns = HashMap::new();
        for (id, thread, cwd) in [
            ("turn-a", "project-a", "/work/Codex Usage"),
            ("turn-b", "project-b", "/tmp/wo-h"),
            ("turn-solo-1", "solo-1", "/tmp/wo-h"),
            ("turn-solo-2", "solo-2", "/tmp/wo-h"),
        ] {
            turns.insert(
                id.into(),
                Turn {
                    id: id.into(),
                    thread: thread.into(),
                    project: cwd.into(),
                    started: now - 200,
                    completed: Some(now - 20),
                    ..Default::default()
                },
            );
        }
        let events = vec![
            attribution_event(
                "a",
                now - 100,
                "project-a",
                "turn-a",
                "/work/Codex Usage",
                false,
                100,
            ),
            attribution_event(
                "b",
                now - 90,
                "project-b",
                "turn-b",
                "/tmp/wo-h",
                false,
                200,
            ),
            attribution_event(
                "child",
                now - 80,
                "child-thread",
                "turn-a",
                "/tmp/gen",
                true,
                50,
            ),
            attribution_event(
                "solo-a",
                now - 70,
                "solo-1",
                "turn-solo-1",
                "/tmp/wo-h",
                false,
                40,
            ),
            attribution_event(
                "solo-b",
                now - 60,
                "solo-2",
                "turn-solo-2",
                "/tmp/wo-h",
                false,
                70,
            ),
            attribution_event(
                "orphan",
                now - 50,
                "child-orphan",
                "missing-root",
                "/tmp/gen",
                true,
                30,
            ),
        ];
        (events, turns, catalog, now)
    }

    fn attribution_chart(titles: &HashMap<String, String>) -> CycleDetails {
        let (events, turns, catalog, now) = attribution_fixture();
        build_dashboard(
            &events,
            &turns,
            titles,
            vec![ResetWindow {
                end: now + 1000,
                seconds: 2000,
            }],
            Coverage::default(),
            Query {
                hours: 24,
                ..Default::default()
            },
            now,
            &catalog,
        )
        .cycle
        .unwrap()
    }

    #[test]
    fn cycle_groups_real_project_across_threads_and_keeps_projectless_threads_separate() {
        let titles = HashMap::from([
            ("solo-1".into(), "同名对话".into()),
            ("solo-2".into(), "同名对话".into()),
        ]);
        let cycle = attribution_chart(&titles);
        let rows = &cycle.attributions;
        assert_eq!(cycle.metrics.total_tokens, 490);
        assert_eq!(rows.len(), 4);
        let project = rows
            .iter()
            .find(|row| row.kind == ShareKind::Project)
            .unwrap();
        assert_eq!(project.id, "d02002a2-80d0-481c-89b9-c972fb22d4ab");
        assert_eq!(project.name, "Codex Usage");
        assert_eq!(project.tokens, 350);
        let conversations: Vec<_> = rows
            .iter()
            .filter(|row| row.kind == ShareKind::Conversation)
            .collect();
        assert_eq!(conversations.len(), 2);
        assert_eq!(conversations[0].name, "同名对话");
        assert_eq!(conversations[1].name, "同名对话");
        assert_eq!(conversations.iter().map(|row| row.tokens).sum::<u64>(), 110);
        assert_ne!(conversations[0].id, conversations[1].id);
        assert_eq!(
            rows.iter()
                .find(|row| row.kind == ShareKind::Unassigned)
                .unwrap()
                .tokens,
            30
        );
        assert_eq!(
            rows.iter().map(|row| row.tokens).sum::<u64>(),
            cycle.metrics.total_tokens
        );
    }

    #[test]
    fn renamed_or_missing_title_changes_only_conversation_label() {
        let old = attribution_chart(&HashMap::from([("solo-1".into(), "旧标题".into())]));
        let renamed = attribution_chart(&HashMap::from([("solo-1".into(), "新标题".into())]));
        let missing = attribution_chart(&HashMap::new());
        fn find(cycle: &CycleDetails) -> &Share {
            cycle
                .attributions
                .iter()
                .find(|row| row.id == "solo-1")
                .unwrap()
        }
        assert_eq!(find(&old).tokens, 40);
        assert_eq!(find(&renamed).tokens, 40);
        assert_eq!(find(&missing).tokens, 40);
        assert_eq!(find(&old).name, "旧标题");
        assert_eq!(find(&renamed).name, "新标题");
        assert_eq!(find(&missing).name, "对话 solo-1");
    }

    #[test]
    fn latest_nonempty_index_title_survives_a_temporary_blank_update() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("codex-usage-titles-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("session_index.jsonl"),
            "{\"id\":\"solo-1\",\"thread_name\":\"旧标题\"}\n{\"id\":\"solo-1\",\"thread_name\":\"新标题\"}\n{\"id\":\"solo-1\",\"thread_name\":\"  \"}\n",
        )
        .unwrap();
        assert_eq!(
            thread_titles(&home).get("solo-1").map(String::as_str),
            Some("新标题")
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn agents_are_unique_instances_not_calls() {
        let rows = [e("a", 100, false), e("b", 101, false), e("c", 102, true)];
        let m = metrics(rows.iter());
        assert_eq!(m.main_agent_count, 1);
        assert_eq!(m.sub_agent_count, 1);
        assert_eq!(m.total_tokens, 330);
    }
    #[test]
    fn raw_standard_and_fast_records_flow_through_parser_and_dashboard_without_token_multipliers() {
        use serde_json::json;
        let home = std::env::temp_dir().join(format!(
            "codex-usage-raw-mode-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sessions = home.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let now = 1_800_000_120;
        for (index, tier) in [(1, "default"), (2, "priority")] {
            let thread = format!("00000000-0000-4000-8000-{index:012}");
            let turn = format!("native-{tier}-turn");
            let records = [
                json!({"timestamp":now-100,"type":"session_meta","payload":{"id":thread}}),
                json!({"timestamp":now-90,"type":"turn_context","payload":{"turn_id":turn,"root_turn_id":turn,"model":"gpt-6.1-sol"}}),
                json!({"timestamp":now-80,"type":"token_usage_record","payload":{"turn_id":turn,"root_turn_id":turn,"thread_id":thread,"response_id":format!("native-{tier}-response"),"service_tier":tier,"usage":{"input_tokens":200000,"cached_input_tokens":100000,"cache_write_input_tokens":80000,"output_tokens":25000}}}),
                json!({"timestamp":now-70,"type":"event_msg","payload":{"type":"task_complete","turn_id":turn}}),
            ];
            fs::write(
                sessions.join(format!("{tier}.jsonl")),
                records
                    .iter()
                    .map(|record| format!("{record}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        }
        let d = AnalyticsStore::default()
            .query(
                &home,
                Query {
                    hours: 24,
                    ..Default::default()
                },
                now,
                None,
            )
            .unwrap();
        assert_eq!(d.coverage.files, 2);
        assert_eq!(d.coverage.malformed_lines, 0);
        assert_eq!(d.history.total_tokens, 450_000);
        assert_eq!(d.history.unpriced_tokens, 0);
        assert!((d.history.cost_usd - 1.5).abs() < 1e-9);
        assert!(d.pricing.missing_models.is_empty());
        for (tier, cost) in [("standard", 0.5), ("fast", 1.0)] {
            let row = d
                .history
                .service_tiers
                .iter()
                .find(|row| row.tier == tier)
                .unwrap();
            assert_eq!(row.tokens, 225_000);
            assert!((row.cost_usd - cost).abs() < 1e-9);
        }
        assert_eq!(d.executions.len(), 2);
        assert_eq!(d.sessions.len(), 2);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn dashboard_groups_missing_modes_as_standard_without_changing_fast_or_costs() {
        let mut rows = vec![
            e("std", 100, false),
            e("fast", 101, false),
            e("unknown", 102, false),
            e("future", 103, false),
        ];
        for row in &mut rows {
            row.tokens = Tokens {
                input_tokens: 100_000,
                cached_input_tokens: 80_000,
                output_tokens: 10_000,
                ..Default::default()
            };
        }
        rows[0].service_tier = Some("default".into());
        rows[1].service_tier = Some("priority".into());
        rows[3].service_tier = Some("unpublished-mode".into());
        let turns = HashMap::from([(
            "t".into(),
            Turn {
                id: "t".into(),
                thread: "main".into(),
                completed: Some(110),
                ..Default::default()
            },
        )]);
        let d = build_dashboard(
            &rows,
            &turns,
            &HashMap::new(),
            vec![ResetWindow {
                end: 604800,
                seconds: 604800,
            }],
            Coverage::default(),
            Query {
                hours: 1,
                ..Default::default()
            },
            120,
            &projects::ProjectCatalog::default(),
        );
        let m = &d.history;
        assert_eq!(m.total_tokens, 440_000);
        assert_eq!(m.unpriced_tokens, 110_000);
        assert!((m.cost_usd - 1.248).abs() < 1e-9);
        assert_eq!(
            m.service_tiers.iter().map(|t| t.tokens).sum::<u64>(),
            m.total_tokens
        );
        assert_eq!(
            m.service_tiers
                .iter()
                .find(|t| t.tier == "fast")
                .unwrap()
                .tokens,
            110_000
        );
        assert_eq!(
            m.service_tiers
                .iter()
                .find(|t| t.tier == "standard")
                .unwrap()
                .tokens,
            220_000
        );
        assert_eq!(m.assumed_standard_tokens, 110_000);
        assert_eq!(
            m.service_tiers
                .iter()
                .find(|t| t.tier == "standard")
                .unwrap()
                .assumed_standard_tokens,
            110_000
        );
        assert!(!m.service_tiers.iter().any(|t| t.tier == "unknown"));
        assert!(rows[2].service_tier.is_none());
        for other in [
            &d.billing,
            &d.cycle.as_ref().unwrap().metrics,
            &d.executions[0].metrics,
            &d.sessions[0].metrics,
        ] {
            assert_eq!(other.cost_usd, m.cost_usd);
            assert_eq!(other.total_tokens, m.total_tokens);
            assert_eq!(other.assumed_standard_tokens, m.assumed_standard_tokens);
            assert!(!other.service_tiers.iter().any(|t| t.tier == "unknown"));
        }
        assert!(
            (d.trend.iter().map(|b| b.metrics.cost_usd).sum::<f64>() - m.cost_usd).abs() < 1e-9
        );
        assert_eq!(d.pricing.missing_models.len(), 1);
        assert_eq!(d.pricing.missing_models[0].tokens, 110_000);
    }
    #[test]
    fn later_explicit_fast_evidence_replaces_the_standard_assumption() {
        let mut event = e("missing", 100, false);
        let standard = metrics([&event].into_iter());
        assert_eq!(standard.assumed_standard_tokens, event.tokens.total());
        assert_eq!(standard.service_tiers[0].tier, "standard");
        assert!(event.service_tier.is_none());
        event.service_tier = Some("priority".into());
        event.request_mode = true;
        let fast = metrics([&event].into_iter());
        assert_eq!(fast.assumed_standard_tokens, 0);
        assert_eq!(fast.service_tiers[0].tier, "fast");
        assert_eq!(fast.total_tokens, standard.total_tokens);
        assert!(fast.cost_usd > standard.cost_usd);
    }
    #[test]
    fn partial_cost_is_reported() {
        let mut event = e("a", 1, false);
        event.model = "unknown".into();
        let m = metrics([&event].into_iter());
        assert_eq!(m.unpriced_tokens, 110);
        assert_eq!(m.cost_usd, 0.0);
    }
    #[test]
    fn manual_reset_truncates_previous_window() {
        let cs = cycles(
            vec![
                ResetWindow {
                    end: 604800,
                    seconds: 604800,
                },
                ResetWindow {
                    end: 704800,
                    seconds: 604800,
                },
            ],
            150000,
        );
        assert_eq!(cs[1].end, 100000);
        assert!(cs[0].ongoing);
    }
    #[test]
    fn range_and_cycles_do_not_change_each_other() {
        let mut old = e("b", 5000, false);
        old.root_turn = "older".into();
        let rows = [e("a", 9000, false), old];
        let turns = HashMap::from([
            (
                "t".into(),
                Turn {
                    id: "t".into(),
                    thread: "main".into(),
                    completed: Some(9000),
                    ..Default::default()
                },
            ),
            (
                "older".into(),
                Turn {
                    id: "older".into(),
                    thread: "main".into(),
                    completed: Some(5000),
                    ..Default::default()
                },
            ),
        ]);
        let q = Query {
            hours: 1,
            cycle_id: None,
            project: None,
            billing_start: None,
            ..Default::default()
        };
        let d = build_dashboard(
            &rows,
            &turns,
            &HashMap::new(),
            vec![ResetWindow {
                end: 604800,
                seconds: 604800,
            }],
            Coverage::default(),
            q,
            10000,
            &projects::ProjectCatalog::default(),
        );
        assert_eq!(d.history.total_tokens, 110);
        assert_eq!(d.cycle.unwrap().metrics.total_tokens, 220);
        assert_eq!(
            d.trend
                .iter()
                .filter(|b| b.start >= d.start)
                .map(|b| b.metrics.total_tokens)
                .sum::<u64>(),
            110
        );
    }
    #[test]
    fn current_event_is_included_in_last_bin() {
        let rows = [e("a", 10000, false)];
        let q = Query {
            hours: 1,
            cycle_id: None,
            project: None,
            billing_start: None,
            ..Default::default()
        };
        let d = build_dashboard(
            &rows,
            &HashMap::from([(
                "t".into(),
                Turn {
                    id: "t".into(),
                    thread: "main".into(),
                    completed: Some(10000),
                    ..Default::default()
                },
            )]),
            &HashMap::new(),
            vec![],
            Coverage::default(),
            q,
            10000,
            &projects::ProjectCatalog::default(),
        );
        assert_eq!(d.trend.last().unwrap().metrics.total_tokens, 110);
    }
    #[test]
    fn completed_execution_includes_children_but_only_once() {
        let rows = [e("a", 100, false), e("b", 101, true)];
        let turn = Turn {
            id: "t".into(),
            thread: "main".into(),
            completed: Some(110),
            started: 90,
            project: "Usage".into(),
            ..Default::default()
        };
        let d = build_dashboard(
            &rows,
            &HashMap::from([("t".into(), turn)]),
            &HashMap::new(),
            vec![],
            Coverage::default(),
            Query {
                hours: 1,
                cycle_id: None,
                project: None,
                billing_start: None,
                ..Default::default()
            },
            120,
            &projects::ProjectCatalog::default(),
        );
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].metrics.total_tokens, 220);
    }
    #[test]
    #[ignore = "reads real local history and diagnostics; explicitly run for mode verification"]
    fn real_mode_recovery_audit() {
        let home = codex_home().unwrap();
        let mut store = AnalyticsStore::default();
        let result = store
            .query(
                &home,
                Query {
                    hours: 24,
                    ..Default::default()
                },
                chrono::Utc::now().timestamp(),
                None,
            )
            .unwrap();
        assert!(result.history.request_mode_tokens > 0);
        assert!(result
            .history
            .service_tiers
            .iter()
            .any(|r| r.tier == "fast" && r.tokens > 0 && r.cost_usd > 0.0));
        assert!(result
            .history
            .service_tiers
            .iter()
            .any(|r| r.tier == "standard" && r.tokens > 0 && r.cost_usd > 0.0));
        assert_eq!(
            result.history.total_tokens,
            result
                .history
                .service_tiers
                .iter()
                .map(|r| r.tokens)
                .sum::<u64>()
        );
        assert_eq!(result.history.unpriced_tokens, 0);
        for m in [&result.history, &result.billing] {
            assert!(!m.service_tiers.iter().any(|r| r.tier == "unknown"));
            let standard = m
                .service_tiers
                .iter()
                .find(|r| r.tier == "standard")
                .unwrap();
            assert_eq!(standard.assumed_standard_tokens, m.assumed_standard_tokens);
            assert!(standard.tokens >= m.assumed_standard_tokens && m.assumed_standard_tokens > 0);
        }
        let summary = serde_json::json!({ "checkedAt": result.updated_at, "files": result.coverage.files, "history": result.history, "billing": result.billing });
        fs::write(
            "/tmp/codex-usage-real-mode-result.json",
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        eprintln!("{summary}");
    }
    #[test]
    #[ignore = "reads local Codex history; emits model totals only"]
    fn local_model_pricing_audit() {
        let mut paths = Vec::new();
        let home = codex_home().unwrap();
        collect_files(&home.join("sessions"), &mut paths, &mut 0);
        collect_files(&home.join("archived_sessions"), &mut paths, &mut 0);
        let mut parsed = Vec::new();
        for path in paths {
            let mut p = ParsedFile::default();
            p.read_append(&path).unwrap();
            parsed.push(p);
        }
        let modern: HashSet<_> = parsed
            .iter()
            .flat_map(|p| p.effective_events())
            .filter(|e| e.modern)
            .map(|e| e.turn.clone())
            .collect();
        let mut seen = HashSet::new();
        let mut models: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for e in parsed.iter().flat_map(|p| p.effective_events()) {
            if (!e.modern && modern.contains(&e.turn))
                || !seen.insert(e.id.clone())
                || ["codex-auto-review", "codex-internal"].contains(&e.model.as_str())
            {
                continue;
            }
            let row = models.entry(e.model.clone()).or_default();
            row.0 += e.tokens.total();
            if pricing::estimate(&e.model, e.tokens).is_none() {
                row.1 += e.tokens.total();
            }
        }
        for (model, (total, unpriced)) in models {
            eprintln!("model={model} tokens={total} unpriced={unpriced}");
        }
    }
    #[test]
    #[ignore = "reads local Codex history; run explicitly for local integration verification"]
    fn local_history_smoke() {
        let mut store = AnalyticsStore::default();
        let home = codex_home().unwrap();
        let now = chrono::Utc::now().timestamp();
        let d = store
            .query(
                &home,
                Query {
                    hours: 24,
                    cycle_id: None,
                    project: None,
                    billing_start: None,
                    ..Default::default()
                },
                now,
                None,
            )
            .unwrap();
        eprintln!("files={} events_since={:?} total24h={} executions={} cycles={} unknown_tokens={} unreadable={} malformed={}",d.coverage.files,d.coverage.first_event_at,d.history.total_tokens,d.executions.len(),d.cycles.len(),d.history.unpriced_tokens,d.coverage.unreadable_files,d.coverage.malformed_lines);
        assert!(d.coverage.files > 0);
        assert!(d.executions.len() <= executions::PAGE_SIZE);
        assert!(d.sessions.len() <= executions::PAGE_SIZE);
        assert!(d.session_page.total <= d.execution_page.total);
        assert!(d.execution_page.total >= d.executions.len());
        assert!(!d.projects.is_empty());
        if let Some(cycle) = &d.cycle {
            assert_eq!(
                cycle.attributions.iter().map(|row| row.tokens).sum::<u64>(),
                cycle.metrics.total_tokens
            );
            let identities: HashSet<_> = cycle
                .attributions
                .iter()
                .map(|row| format!("{:?}:{}", row.kind, row.id))
                .collect();
            assert_eq!(identities.len(), cycle.attributions.len());
        }
        let again = store
            .query(
                &home,
                Query {
                    hours: 24,
                    cycle_id: None,
                    project: None,
                    billing_start: None,
                    ..Default::default()
                },
                now,
                None,
            )
            .unwrap();
        assert_eq!(d.history.total_tokens, again.history.total_tokens);
        let independent = store
            .query(
                &home,
                Query {
                    hours: 24,
                    project_scope: Some("projectless".into()),
                    execution_hours: Some(168),
                    page: Some(2),
                    ..Default::default()
                },
                now,
                None,
            )
            .unwrap();
        assert_eq!(independent.history.total_tokens, d.history.total_tokens);
        assert!(independent
            .executions
            .iter()
            .all(|e| e.project_id.is_none()));
        eprintln!(
            "completed_total={} projects={} conversations={} projectless_week={} page={}",
            d.execution_page.total,
            d.projects.len(),
            d.conversations.len(),
            independent.execution_page.total,
            independent.execution_page.page
        );
    }
}
