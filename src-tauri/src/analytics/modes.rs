//! Recover per-turn request modes from Codex's local diagnostic tags.
//! Only exact thread + turn + model matches are used. No prompt/body is retained.
use super::{pricing::normalize_tier, types::UsageEvent};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PAGE_SIZE: usize = 256;
const MAX_OUTPUT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Evidence {
    thread: String,
    turn: String,
    model: String,
    tier: String,
    first_at: i64,
    conflicted: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    source: String,
    last_id: i64,
    records: HashMap<String, Evidence>,
}

#[derive(Deserialize)]
struct DiagnosticRow {
    id: i64,
    ts: i64,
    thread_id: Option<String>,
    body: String,
}

#[derive(Default)]
pub struct ModeEvidence {
    cache: Cache,
    path: Option<PathBuf>,
    checked: Option<Instant>,
}

fn valid_id(value: &str) -> bool {
    value.len() == 36 && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}
fn valid_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn key(thread: &str, turn: &str, model: &str) -> String {
    format!("{thread}:{turn}:{model}")
}

// Trace spans may repeat a field. Ambiguous identities never produce a match.
fn trace_field(prefix: &str, field: &str) -> Option<String> {
    let needle = format!("{field}=");
    let mut result: Option<String> = None;
    for (offset, _) in prefix.match_indices(&needle) {
        if offset > 0 && prefix.as_bytes()[offset - 1].is_ascii_alphanumeric()
            || offset > 0 && prefix.as_bytes()[offset - 1] == b'_'
        {
            continue;
        }
        let rest = prefix[offset + needle.len()..].trim_start_matches('"');
        let value: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || "._-".contains(*c))
            .collect();
        if value.is_empty() || result.as_ref().is_some_and(|prior| prior != &value) {
            return None;
        }
        result = Some(value);
    }
    result
}

fn parse_row(row: &DiagnosticRow) -> Option<Evidence> {
    let thread = row.thread_id.as_ref()?;
    if !valid_id(thread) || !row.body.starts_with("session_loop{") {
        return None;
    }
    let (prefix, tags) = row.body.rsplit_once("tags_json=")?;
    if trace_field(prefix, "thread_id").as_ref() != Some(thread) {
        return None;
    }
    let turn = trace_field(prefix, "turn_id")?;
    let model = trace_field(prefix, "model")?;
    if !valid_id(&turn) || !valid_model(&model) {
        return None;
    }
    let tags: serde_json::Value = serde_json::from_str(tags).ok()?;
    let tier = normalize_tier(tags.get("service_tier")?.as_str());
    if !["standard", "fast", "ultrafast"].contains(&tier.as_str()) {
        return None;
    }
    Some(Evidence {
        thread: thread.clone(),
        turn,
        model,
        tier,
        first_at: row.ts,
        conflicted: false,
    })
}

impl ModeEvidence {
    pub fn configure_cache(&mut self, path: PathBuf) {
        if let Ok(bytes) = fs::read(&path) {
            if bytes.len() <= MAX_OUTPUT as usize {
                if let Ok(cache) = serde_json::from_slice::<Cache>(&bytes) {
                    if cache.version == 1
                        && cache.records.len() <= 100_000
                        && cache.records.iter().all(|(k, e)| {
                            valid_id(&e.thread)
                                && valid_id(&e.turn)
                                && valid_model(&e.model)
                                && k == &key(&e.thread, &e.turn, &e.model)
                                && ["standard", "fast", "ultrafast"].contains(&e.tier.as_str())
                        })
                    {
                        self.cache = cache;
                    }
                }
            }
        }
        self.path = Some(path);
    }

    fn add(&mut self, record: Evidence) {
        let k = key(&record.thread, &record.turn, &record.model);
        if let Some(prior) = self.cache.records.get_mut(&k) {
            prior.conflicted |= prior.tier != record.tier;
            prior.first_at = prior.first_at.min(record.first_at);
        } else if self.cache.records.len() < 100_000 {
            self.cache.records.insert(k, record);
        }
    }

    pub fn recover(&self, event: &mut UsageEvent) {
        // A response's own service_tier (including default downgrade) always wins.
        if event.service_tier.is_some() {
            return;
        }
        if let Some(record) = self
            .cache
            .records
            .get(&key(&event.thread, &event.turn, &event.model))
        {
            if !record.conflicted && record.first_at <= event.at {
                event.service_tier = Some(record.tier.clone());
                event.request_mode = true;
            }
        }
    }

    pub fn refresh(&mut self, history: &Path) {
        if self
            .checked
            .is_some_and(|at| at.elapsed() < Duration::from_secs(15))
        {
            return;
        }
        self.checked = Some(Instant::now());
        let database = history.join("logs_2.sqlite");
        if !database.is_file()
            || fs::symlink_metadata(&database).is_ok_and(|m| m.file_type().is_symlink())
        {
            return;
        }
        let source = database.to_string_lossy().to_string();
        if self.cache.source != source {
            self.cache.source = source;
            self.cache.last_id = 0;
        }
        let Ok(maximum) = sqlite(&database, "select coalesce(max(id),0) as max_id from logs;")
        else {
            return;
        };
        let maximum = maximum
            .first()
            .and_then(|v| v["max_id"].as_i64())
            .unwrap_or(0);
        if maximum < self.cache.last_id {
            self.cache.last_id = 0;
        }
        if maximum == self.cache.last_id {
            return;
        }
        let mut changed = false;
        for _ in 0..16 {
            let sql = format!("select id,ts,thread_id,feedback_log_body as body from logs where id>{} and target='feedback_tags' and instr(feedback_log_body,'tags_json=')>0 and instr(feedback_log_body,'service_tier')>0 order by id limit {PAGE_SIZE};", self.cache.last_id);
            let Ok(values) = sqlite(&database, &sql) else {
                break;
            };
            let count = values.len();
            let prior_id = self.cache.last_id;
            for value in values {
                let Ok(row) = serde_json::from_value::<DiagnosticRow>(value) else {
                    continue;
                };
                self.cache.last_id = self.cache.last_id.max(row.id);
                if let Some(record) = parse_row(&row) {
                    self.add(record);
                }
                changed = true;
            }
            if count < PAGE_SIZE {
                self.cache.last_id = self.cache.last_id.max(maximum);
                changed = true;
                break;
            }
            if prior_id == self.cache.last_id {
                break;
            }
        }
        if changed {
            self.save();
        }
    }

    fn save(&mut self) {
        self.cache.version = 1;
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&self.cache) else {
            return;
        };
        if bytes.len() > MAX_OUTPUT as usize || fs::create_dir_all(parent).is_err() {
            return;
        }
        let temporary = path.with_extension("json.tmp");
        if fs::write(&temporary, bytes).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600));
        }
        let _ = fs::rename(temporary, path);
    }
}

// System SQLite runs locally and read-only. SQL is fixed; no shell or chat data
// is used as a command. Bound output and execution time, and fail to unknown.
fn sqlite(database: &Path, sql: &str) -> Result<Vec<serde_json::Value>, ()> {
    let mut child = Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json", "-cmd", ".timeout 1000"])
        .arg(database)
        .arg(sql)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    let stdout = child.stdout.take().ok_or(())?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(());
        }
        Ok(bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader.join().map_err(|_| ())??;
    if !status.is_some_and(|s| s.success()) {
        return Err(());
    }
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    const THREAD: &str = "00000000-0000-4000-8000-000000000001";
    const TURN: &str = "00000000-0000-4000-8000-000000000002";
    fn row(tier: &str) -> DiagnosticRow {
        DiagnosticRow { id: 1, ts: 10, thread_id: Some(THREAD.into()),
            body: format!("session_loop{{thread_id={THREAD}}}:run_turn{{turn_id={TURN} model=gpt-6-astra}}: model=gpt-6-astra tags_json={{\"service_tier\":\"{tier}\"}}") }
    }
    fn event() -> UsageEvent {
        UsageEvent {
            id: "response:actual-id".into(),
            at: 20,
            thread: THREAD.into(),
            turn: TURN.into(),
            root_turn: TURN.into(),
            parent: None,
            project: "".into(),
            model: "gpt-6-astra".into(),
            service_tier: None,
            request_mode: false,
            is_subagent: false,
            tokens: super::super::types::Tokens {
                input_tokens: 100,
                ..Default::default()
            },
            modern: true,
        }
    }
    #[test]
    fn exact_diagnostic_match_recovers_priority_without_changing_usage() {
        let mut evidence = ModeEvidence::default();
        evidence.add(parse_row(&row("priority")).unwrap());
        let mut e = event();
        evidence.recover(&mut e);
        assert_eq!(e.service_tier.as_deref(), Some("fast"));
        assert!(e.request_mode);
        assert_eq!(e.tokens.total(), 100);
        let mut other_model = event();
        other_model.model = "other-model".into();
        evidence.recover(&mut other_model);
        assert!(other_model.service_tier.is_none());
        let mut earlier = event();
        earlier.at = 9;
        evidence.recover(&mut earlier);
        assert!(earlier.service_tier.is_none());
        let mut another = event();
        another.turn = THREAD.into();
        evidence.recover(&mut another);
        assert!(another.service_tier.is_none());
        let mut another = event();
        another.thread = TURN.into();
        evidence.recover(&mut another);
        assert!(another.service_tier.is_none());
    }
    #[test]
    fn response_downgrade_and_conflicting_traces_never_get_overwritten() {
        let mut evidence = ModeEvidence::default();
        evidence.add(parse_row(&row("priority")).unwrap());
        let mut actual = event();
        actual.service_tier = Some("default".into());
        evidence.recover(&mut actual);
        assert_eq!(actual.service_tier.as_deref(), Some("default"));
        assert!(!actual.request_mode);
        evidence.add(parse_row(&row("default")).unwrap());
        let mut conflict = event();
        evidence.recover(&mut conflict);
        assert!(conflict.service_tier.is_none());
    }
    #[test]
    fn malformed_or_ambiguous_trace_identity_stays_unknown() {
        for value in ["future", "", "unknown"] {
            assert!(parse_row(&row(value)).is_none());
        }
        let mut bad = row("fast");
        bad.body = bad
            .body
            .replace("model=gpt-6-astra tags", "model=other tags");
        assert!(parse_row(&bad).is_none());
        let mut bad = row("fast");
        bad.thread_id = Some(TURN.into());
        assert!(parse_row(&bad).is_none());
        let mut bad = row("fast");
        bad.body.push_str(" truncated");
        assert!(parse_row(&bad).is_none());
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn reads_sqlite_incrementally_and_persists_only_mode_evidence() {
        let dir =
            std::env::temp_dir().join(format!("codex-mode-sqlite-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("logs_2.sqlite");
        let create = "create table logs(id integer primary key,ts integer,target text,thread_id text,feedback_log_body text);";
        assert!(Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg(create)
            .status()
            .unwrap()
            .success());
        let body = row("priority").body.replace('\'', "''");
        let sql = format!("insert into logs values(1,10,'feedback_tags','{THREAD}','{body}');");
        assert!(Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg(sql)
            .status()
            .unwrap()
            .success());
        let original = fs::read(&db).unwrap();
        let mut evidence = ModeEvidence::default();
        evidence.configure_cache(dir.join("evidence.json"));
        evidence.refresh(&dir);
        let mut e = event();
        evidence.recover(&mut e);
        assert_eq!(e.service_tier.as_deref(), Some("fast"));
        assert_eq!(fs::read(&db).unwrap(), original);
        assert_eq!(evidence.cache.last_id, 1);
        let sql = format!(
            "insert into logs values(2,11,'feedback_tags','{THREAD}','{}');",
            row("default").body.replace('\'', "''")
        );
        assert!(Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg(sql)
            .status()
            .unwrap()
            .success());
        evidence.checked = None;
        evidence.refresh(&dir);
        let mut conflict = event();
        evidence.recover(&mut conflict);
        assert!(conflict.service_tier.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sanitized_cache_survives_source_log_removal() {
        let dir =
            std::env::temp_dir().join(format!("codex-mode-cache-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("modes.json");
        let mut original = ModeEvidence::default();
        original.configure_cache(path.clone());
        original.add(parse_row(&row("fast")).unwrap());
        original.save();
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("tags_json"));
        assert!(!bytes.contains("feedback_log_body"));
        let mut recovered = ModeEvidence::default();
        recovered.configure_cache(path);
        recovered.refresh(&dir);
        let mut e = event();
        recovered.recover(&mut e);
        assert_eq!(e.service_tier.as_deref(), Some("fast"));
        fs::remove_dir_all(dir).unwrap();
    }
}
