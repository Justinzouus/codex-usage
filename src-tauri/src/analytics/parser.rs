use super::types::{ResetWindow, Tokens, Turn, UsageEvent};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

const MAX_LINE: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct ParsedFile {
    pub offset: u64,
    pub events: Vec<UsageEvent>,
    pub turns: HashMap<String, Turn>,
    pub windows: Vec<ResetWindow>,
    pub malformed: usize,
    pub oversized: usize,
    thread: String,
    parent: Option<String>,
    project: String,
    model: String,
    service_tier: Option<String>,
    current_turn: String,
    root_turn: String,
    subagent: bool,
    internal: bool,
    cumulative: Tokens,
    modern_turns: HashSet<String>,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
fn service_tier(v: &Value) -> Option<String> {
    v.get("service_tier")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(str::to_owned)
}

fn usage_tokens(v: &Value) -> Result<Tokens, serde_json::Error> {
    let mut tokens: Tokens = serde_json::from_value(v.clone())?;
    let details = &v["input_tokens_details"];
    let ttl = &details["cache_write_tokens_details"];
    // Rollouts can contain the legacy flat counters and the API's nested details.
    // They describe the same tokens; choose a total rather than adding both.
    tokens.cached_input_tokens = v["cached_input_tokens"]
        .as_u64()
        .or_else(|| details["cached_tokens"].as_u64())
        .or_else(|| details["cache_read_tokens"].as_u64())
        .unwrap_or_default();
    tokens.cache_write_12h_input_tokens = v["cache_write_12h_input_tokens"]
        .as_u64()
        .or_else(|| ttl["ttl_12h_tokens"].as_u64())
        .unwrap_or_default();
    tokens.cache_write_input_tokens = v["cache_write_input_tokens"]
        .as_u64()
        .or_else(|| v["cache_write_tokens"].as_u64())
        .or_else(|| details["cache_write_tokens"].as_u64())
        .unwrap_or_else(|| {
            ttl["ttl_30m_tokens"]
                .as_u64()
                .unwrap_or_default()
                .saturating_add(tokens.cache_write_12h_input_tokens)
        });
    // Missing TTL details mean the existing 30-minute write rate. Preserve an
    // invalid 12-hour subtotal so pricing can reject it instead of hiding it.
    Ok(tokens)
}

fn timestamp(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| {
        v.as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp())
    })
}
fn project_name(cwd: &str) -> String {
    if cwd.is_empty() {
        "未归属项目".into()
    } else {
        cwd.to_string()
    }
}

impl ParsedFile {
    pub fn read_append(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        if file.metadata()?.len() < self.offset {
            *self = Self::default();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::with_capacity(128 * 1024, file);
        // Bounded line buffering; very large tool results never accumulate in memory.
        let mut line = Vec::new();
        loop {
            let mut consumed = 0u64;
            let mut oversized = false;
            let mut complete = false;
            loop {
                let available = reader.fill_buf()?;
                if available.is_empty() {
                    break;
                }
                let end = memchr::memchr(b'\n', available)
                    .map(|i| i + 1)
                    .unwrap_or(available.len());
                complete = available[end - 1] == b'\n';
                if !oversized {
                    if line.len() + end <= MAX_LINE {
                        line.extend_from_slice(&available[..end]);
                    } else {
                        line.clear();
                        oversized = true;
                    }
                }
                consumed += end as u64;
                reader.consume(end);
                if complete {
                    break;
                }
            }
            if !complete {
                break;
            } // Keep a partially written final line for the next refresh.
            self.offset += consumed;
            if oversized {
                self.oversized += 1;
            } else if let Ok(raw) = std::str::from_utf8(&line) {
                let head = raw.get(..raw.len().min(512)).unwrap_or(raw);
                let relevant = [
                    "session_meta",
                    "turn_context",
                    "token_usage_record",
                    "task_started",
                    "task_complete",
                    "token_count",
                ]
                .iter()
                .any(|kind| {
                    head.contains(&format!("\"type\":\"{kind}\""))
                        || head.contains(&format!("\"type\": \"{kind}\""))
                }) || head.contains("\"role\":\"user\"")
                    || head.contains("\"role\": \"user\"");
                if relevant {
                    match serde_json::from_str::<Value>(raw) {
                        Ok(v) => self.accept(&v),
                        Err(_) => self.malformed += 1,
                    }
                }
            } else {
                self.malformed += 1;
            }
            line.clear();
        }
        Ok(())
    }

    pub fn effective_events(&self) -> impl Iterator<Item = &UsageEvent> {
        self.events
            .iter()
            .filter(|e| e.modern || !self.modern_turns.contains(&e.turn))
    }

    fn ensure_turn(&mut self, id: &str, at: i64) {
        self.turns.entry(id.to_string()).or_insert_with(|| Turn {
            id: id.into(),
            thread: self.thread.clone(),
            root_turn: self.root_turn.clone(),
            project: self.project.clone(),
            started: at,
            is_subagent: self.subagent,
            ..Default::default()
        });
    }

    fn accept(&mut self, v: &Value) {
        let p = &v["payload"];
        let at = timestamp(&v["timestamp"]).unwrap_or(0);
        match v["type"].as_str().unwrap_or("") {
            "session_meta" => {
                self.thread = text(p, "id");
                if self.thread.is_empty() {
                    self.thread = text(p, "session_id");
                }
                self.parent = p["parent_thread_id"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| {
                        p.pointer("/source/subagent/spawn/parent_thread_id")
                            .and_then(Value::as_str)
                            .map(String::from)
                    });
                self.subagent =
                    p["thread_source"] == "subagent" || p["source"].get("subagent").is_some();
                self.internal =
                    p.pointer("/source/subagent/other").and_then(Value::as_str) == Some("guardian");
                self.project = project_name(&text(p, "cwd"));
            }
            "turn_context" => {
                let id = text(p, "turn_id");
                if !id.is_empty() {
                    self.current_turn = id;
                }
                self.root_turn = text(p, "root_turn_id");
                self.model = text(p, "model");
                self.service_tier = service_tier(p);
                if p["cwd"].is_string() {
                    self.project = project_name(&text(p, "cwd"));
                }
                let id = self.current_turn.clone();
                self.ensure_turn(&id, at);
                if let Some(turn) = self.turns.get_mut(&id) {
                    turn.project = self.project.clone();
                    if !self.root_turn.is_empty() {
                        turn.root_turn = self.root_turn.clone();
                    }
                }
            }
            "response_item" if p["type"] == "message" && p["role"] == "user" => {
                if self.internal {
                    return;
                }
                let id = self.current_turn.clone();
                if id.is_empty() {
                    return;
                }
                // Only a short local title is retained; no full prompt or assistant/tool content.
                let title = p["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part["text"].as_str())
                    .find(|s| !s.trim_start().starts_with('<'))
                    .map(|s| {
                        s.lines()
                            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                            .unwrap_or(s)
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(90)
                            .collect::<String>()
                    });
                if let Some(title) = title {
                    self.ensure_turn(&id, at);
                    if let Some(t) = self.turns.get_mut(&id) {
                        if t.title.is_empty() {
                            t.title = title;
                        }
                    }
                }
            }
            "token_usage_record" => {
                let tokens = usage_tokens(&p["usage"]).unwrap_or_default();
                let turn = text(p, "turn_id");
                if turn.is_empty() {
                    return;
                }
                self.modern_turns.insert(turn.clone());
                let response = text(p, "response_id");
                let id = if response.is_empty() {
                    format!("record:{turn}:{at}:{}", p["turn_token_usage"])
                } else {
                    format!("response:{response}")
                };
                let root = text(p, "root_turn_id");
                let thread = text(p, "thread_id");
                // A response's actual tier takes precedence over the requested turn tier:
                // Fast requests can be downgraded to default by the service.
                let tier = service_tier(p).or_else(|| {
                    (turn == self.current_turn)
                        .then(|| self.service_tier.clone())
                        .flatten()
                });
                let requested = service_tier(p).is_none() && tier.is_some();
                self.push_event(id, at, turn, root, thread, tokens, true, tier, requested);
            }
            "event_msg" => match p["type"].as_str().unwrap_or("") {
                "task_started" => {
                    self.current_turn = text(p, "turn_id");
                    self.service_tier = None;
                    self.root_turn = text(p, "root_turn_id");
                    let id = self.current_turn.clone();
                    let started = timestamp(&p["started_at"]).unwrap_or(at);
                    self.ensure_turn(&id, started);
                }
                "task_complete" => {
                    let id = text(p, "turn_id");
                    self.ensure_turn(&id, at);
                    if let Some(t) = self.turns.get_mut(&id) {
                        t.completed = timestamp(&p["completed_at"]).or(Some(at));
                        if let Some(s) = timestamp(&p["started_at"]) {
                            t.started = s;
                        }
                    }
                }
                "token_count" => {
                    self.read_limits(&p["rate_limits"]);
                    if let Ok(total) = usage_tokens(&p["info"]["total_token_usage"]) {
                        let delta = if total.input_tokens < self.cumulative.input_tokens
                            || total.output_tokens < self.cumulative.output_tokens
                        {
                            usage_tokens(&p["info"]["last_token_usage"]).unwrap_or_default()
                        } else {
                            total.delta(self.cumulative)
                        };
                        self.cumulative = total;
                        let turn = self.current_turn.clone();
                        let id = format!(
                            "legacy:{turn}:{at}:{}:{}",
                            total.input_tokens, total.output_tokens
                        );
                        self.push_event(
                            id,
                            at,
                            turn,
                            self.root_turn.clone(),
                            self.thread.clone(),
                            delta,
                            false,
                            self.service_tier.clone(),
                            self.service_tier.is_some(),
                        );
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn push_event(
        &mut self,
        id: String,
        at: i64,
        turn: String,
        root: String,
        thread: String,
        tokens: Tokens,
        modern: bool,
        service_tier: Option<String>,
        request_mode: bool,
    ) {
        if tokens.total() == 0 || at <= 0 {
            return;
        }
        self.events.push(UsageEvent {
            id,
            at,
            turn,
            root_turn: root,
            thread: if thread.is_empty() {
                self.thread.clone()
            } else {
                thread
            },
            parent: self.parent.clone(),
            project: self.project.clone(),
            model: if self.internal {
                "codex-internal".into()
            } else {
                self.model.clone()
            },
            is_subagent: self.subagent,
            tokens,
            modern,
            service_tier,
            request_mode,
        });
    }

    fn read_limits(&mut self, limits: &Value) {
        if self.internal {
            return;
        }
        if let Some(id) = limits["limit_id"].as_str() {
            if id != "codex" {
                return;
            }
        }
        for key in ["primary", "secondary"] {
            let w = &limits[key];
            let mins = w["window_minutes"].as_i64().unwrap_or(0);
            if (10_079..=10_081).contains(&mins) {
                if let Some(end) = w["resets_at"].as_i64() {
                    if !self.windows.iter().any(|v| v.end == end) {
                        self.windows.push(ResetWindow {
                            end,
                            seconds: mins * 60,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(p: &mut ParsedFile, kind: &str, payload: Value) {
        p.accept(&json!({"timestamp":"2026-09-21T12:00:00Z","type":kind,"payload":payload}));
    }
    fn setup() -> ParsedFile {
        let mut p = ParsedFile::default();
        event(
            &mut p,
            "session_meta",
            json!({"id":"thread","cwd":"/projects/Usage","source":"vscode"}),
        );
        event(
            &mut p,
            "event_msg",
            json!({"type":"task_started","turn_id":"one"}),
        );
        event(
            &mut p,
            "turn_context",
            json!({"turn_id":"one","model":"gpt-5.6-sol"}),
        );
        p
    }
    fn count(p: &mut ParsedFile, input: u64, output: u64) {
        event(
            p,
            "event_msg",
            json!({"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":input/2,"output_tokens":output},"last_token_usage":{"input_tokens":input,"output_tokens":output}}}),
        );
    }
    #[test]
    fn legacy_cumulative_duplicate_is_zero() {
        let mut p = setup();
        count(&mut p, 100, 10);
        count(&mut p, 100, 10);
        count(&mut p, 200, 25);
        assert_eq!(
            p.effective_events().map(|e| e.tokens.total()).sum::<u64>(),
            225
        );
    }
    #[test]
    fn modern_replaces_mirrored_legacy_in_the_same_turn() {
        let mut p = setup();
        event(
            &mut p,
            "token_usage_record",
            json!({"turn_id":"one","thread_id":"thread","response_id":"r1","usage":{"input_tokens":100,"output_tokens":10}}),
        );
        count(&mut p, 100, 10);
        assert_eq!(p.effective_events().count(), 1);
        assert_eq!(p.effective_events().next().unwrap().tokens.total(), 110);
    }
    #[test]
    fn cumulative_survives_turn_changes() {
        let mut p = setup();
        count(&mut p, 100, 10);
        event(
            &mut p,
            "event_msg",
            json!({"type":"task_started","turn_id":"two"}),
        );
        count(&mut p, 250, 30);
        assert_eq!(p.effective_events().last().unwrap().tokens.total(), 170);
    }
    #[test]
    fn fast_mode_preserves_actual_tokens_and_response_tier_wins() {
        let mut p = setup();
        event(
            &mut p,
            "turn_context",
            json!({"turn_id":"one","model":"gpt-6-astra","service_tier":"fast"}),
        );
        event(
            &mut p,
            "token_usage_record",
            json!({"turn_id":"one","response_id":"fast","service_tier":"priority","usage":{"input_tokens":100,"output_tokens":10}}),
        );
        event(
            &mut p,
            "token_usage_record",
            json!({"turn_id":"one","response_id":"downgraded","service_tier":"default","usage":{"input_tokens":100,"output_tokens":10}}),
        );
        assert_eq!(p.events[0].service_tier.as_deref(), Some("priority"));
        assert_eq!(p.events[1].service_tier.as_deref(), Some("default"));
        assert_eq!(p.events.iter().map(|e| e.tokens.total()).sum::<u64>(), 220);
    }
    #[test]
    fn missing_tier_stays_unknown_and_does_not_leak_between_turns() {
        let mut p = setup();
        count(&mut p, 100, 10);
        assert_eq!(p.events[0].service_tier, None);
        event(
            &mut p,
            "turn_context",
            json!({"turn_id":"one","model":"gpt-6-astra","service_tier":"fast"}),
        );
        count(&mut p, 200, 20);
        assert_eq!(p.events[1].service_tier.as_deref(), Some("fast"));
        event(
            &mut p,
            "event_msg",
            json!({"type":"task_started","turn_id":"two"}),
        );
        count(&mut p, 300, 30);
        assert_eq!(p.events[2].service_tier, None);
        event(
            &mut p,
            "turn_context",
            json!({"turn_id":"two","model":"gpt-6-astra","service_tier":"ultrafast"}),
        );
        event(
            &mut p,
            "token_usage_record",
            json!({"turn_id":"one","response_id":"late","usage":{"input_tokens":100,"output_tokens":10}}),
        );
        assert_eq!(p.events.last().unwrap().service_tier, None);
        event(
            &mut p,
            "turn_context",
            json!({"turn_id":"three","model":"gpt-6-astra"}),
        );
        count(&mut p, 400, 40);
        assert_eq!(p.events.last().unwrap().service_tier, None);
    }
    #[test]
    fn reasoning_is_already_part_of_output() {
        let t:Tokens=serde_json::from_value(json!({"input_tokens":100,"cached_input_tokens":50,"output_tokens":20,"reasoning_output_tokens":15,"total_tokens":120})).unwrap();
        assert_eq!(t.total(), 120);
    }
    #[test]
    fn modern_api_cache_ttl_details_are_subsets_not_extra_tokens() {
        let mut p = setup();
        event(
            &mut p,
            "token_usage_record",
            json!({"turn_id":"one","response_id":"ttl","usage":{
                "input_tokens":1000,"output_tokens":20,
                "input_tokens_details":{
                    "cached_tokens":400,"cache_write_tokens":300,
                    "cache_write_tokens_details":{"ttl_30m_tokens":100,"ttl_12h_tokens":200}
                }
            }}),
        );
        let tokens = p.events[0].tokens;
        assert_eq!(tokens.cached_input_tokens, 400);
        assert_eq!(tokens.cache_write_input_tokens, 300);
        assert_eq!(tokens.cache_write_12h_input_tokens, 200);
        assert_eq!(tokens.total(), 1020);
    }
    #[test]
    fn ttl_details_supply_missing_write_total_and_flat_counters_take_precedence() {
        let details = json!({
            "input_tokens":1000,"output_tokens":20,
            "input_tokens_details":{
                "cache_write_tokens_details":{"ttl_30m_tokens":100,"ttl_12h_tokens":200}
            }
        });
        let tokens = usage_tokens(&details).unwrap();
        assert_eq!(tokens.cache_write_input_tokens, 300);
        assert_eq!(tokens.cache_write_12h_input_tokens, 200);
        let mut mirrored = details;
        mirrored["cache_write_input_tokens"] = json!(300);
        mirrored["cache_write_tokens"] = json!(300);
        mirrored["input_tokens_details"]["cache_write_tokens"] = json!(300);
        assert_eq!(usage_tokens(&mirrored).unwrap(), tokens);
        let legacy = usage_tokens(&json!({"input_tokens":1000,"cache_write_tokens":300})).unwrap();
        assert_eq!(legacy.cache_write_input_tokens, 300);
        assert_eq!(legacy.cache_write_12h_input_tokens, 0);
        mirrored["cache_write_input_tokens"] = json!(0);
        let invalid = usage_tokens(&mirrored).unwrap();
        assert_eq!(invalid.cache_write_input_tokens, 0);
        assert_eq!(invalid.cache_write_12h_input_tokens, 200);
    }
    #[test]
    fn legacy_cumulative_ttl_details_delta_and_reset_do_not_double_count() {
        let mut p = setup();
        let usage = |input, write_30m, write_12h| {
            json!({"input_tokens":input,"output_tokens":10,"input_tokens_details":{
                "cache_write_tokens_details":{"ttl_30m_tokens":write_30m,"ttl_12h_tokens":write_12h}
            }})
        };
        for total in [usage(100, 10, 20), usage(200, 20, 40), usage(200, 20, 40)] {
            event(
                &mut p,
                "event_msg",
                json!({"type":"token_count","info":{"total_token_usage":total}}),
            );
        }
        assert_eq!(p.events.len(), 2);
        assert_eq!(p.events[1].tokens.input_tokens, 100);
        assert_eq!(p.events[1].tokens.cache_write_input_tokens, 30);
        assert_eq!(p.events[1].tokens.cache_write_12h_input_tokens, 20);
        event(
            &mut p,
            "event_msg",
            json!({"type":"token_count","info":{
                "total_token_usage":usage(50, 2, 3),"last_token_usage":usage(50, 2, 3)
            }}),
        );
        let reset = p.events.last().unwrap().tokens;
        assert_eq!(reset.cache_write_input_tokens, 5);
        assert_eq!(reset.cache_write_12h_input_tokens, 3);
        assert_eq!(p.events.iter().map(|e| e.tokens.total()).sum::<u64>(), 270);
    }
    #[test]
    fn weekly_window_can_be_primary() {
        let mut p = setup();
        p.read_limits(
            &json!({"primary":{"window_minutes":10080,"resets_at":1800000000},"secondary":null}),
        );
        assert_eq!(p.windows.len(), 1);
    }
    #[test]
    fn partial_line_is_only_consumed_after_newline() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "codex-usage-parser-{}-partial.jsonl",
            std::process::id()
        ));
        let complete="{\"timestamp\":\"2026-09-21T12:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test\"}}";
        std::fs::write(&path, complete).unwrap();
        let mut p = ParsedFile::default();
        p.read_append(&path).unwrap();
        assert_eq!(p.offset, 0);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        p.read_append(&path).unwrap();
        assert_eq!(p.thread, "test");
        assert_eq!(p.offset, (complete.len() + 1) as u64);
        std::fs::remove_file(path).unwrap();
    }
}
