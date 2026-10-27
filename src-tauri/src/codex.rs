use std::{collections::HashMap, ffi::OsString, path::PathBuf, process::Stdio, time::Duration};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    time::timeout,
};

use crate::models::{ProviderSnapshot, UsageWindow};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const RATE_LIMIT_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct QueryError {
    status: &'static str,
    message: &'static str,
}

impl QueryError {
    const fn unavailable(message: &'static str) -> Self {
        Self {
            status: "unavailable",
            message,
        }
    }

    const fn signed_out(message: &'static str) -> Self {
        Self {
            status: "signed_out",
            message,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LimitsResponse {
    rate_limits: LimitSnapshot,
    #[serde(default)]
    rate_limits_by_limit_id: Option<HashMap<String, LimitSnapshot>>,
    #[serde(default)]
    rate_limit_reset_credits: Option<ResetCredits>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LimitSnapshot {
    #[serde(default)]
    limit_id: Option<String>,
    #[serde(default)]
    primary: Option<RateWindow>,
    #[serde(default)]
    secondary: Option<RateWindow>,
    #[serde(default)]
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateWindow {
    used_percent: i32,
    #[serde(default)]
    window_duration_mins: Option<i64>,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredits {
    available_count: i64,
    #[serde(default)]
    credits: Option<Vec<ResetCredit>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredit {
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

fn codex_candidates() -> Vec<OsString> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("CODEX_CLI_PATH") {
        candidates.push(path);
    }

    #[cfg(target_os = "macos")]
    {
        for path in [
            "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            "/Applications/ChatGPT.app/Contents/Resources/codex",
            "/Applications/Codex.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            "/Applications/Codex.app/Contents/Resources/codex",
            "/opt/homebrew/bin/codex",
            "/usr/local/bin/codex",
        ] {
            if PathBuf::from(path).is_file() {
                candidates.push(path.into());
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let standalone = PathBuf::from(home).join(".codex/packages/standalone/current/codex");
            if standalone.is_file() {
                candidates.push(standalone.into_os_string());
            }
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let bundled = PathBuf::from(local_app_data).join("Programs/ChatGPT/resources/codex.exe");
        if bundled.is_file() {
            candidates.push(bundled.into_os_string());
        }
    }

    candidates.push("codex".into());
    candidates
}

async fn spawn_app_server() -> Result<Child, QueryError> {
    for candidate in codex_candidates() {
        let mut command = Command::new(candidate);
        command
            .args([
                "app-server",
                "--disable",
                "plugins",
                "--disable",
                "remote_plugin",
                "--disable",
                "plugin_sharing",
                "--disable",
                "apps",
                "-c",
                "analytics.enabled=false",
                "--stdio",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(QueryError::unavailable(
                    "Codex could not be started. Try updating or reinstalling Codex.",
                ))
            }
        }
    }
    Err(QueryError::unavailable(
        "Codex CLI was not found. Install Codex or set CODEX_CLI_PATH.",
    ))
}

async fn write_message(stdin: &mut ChildStdin, value: Value) -> Result<(), QueryError> {
    let mut payload = serde_json::to_vec(&value)
        .map_err(|_| QueryError::unavailable("Could not encode the Codex request."))?;
    payload.push(b'\n');
    stdin
        .write_all(&payload)
        .await
        .map_err(|_| QueryError::unavailable("Codex app-server stopped unexpectedly."))?;
    stdin
        .flush()
        .await
        .map_err(|_| QueryError::unavailable("Codex app-server stopped unexpectedly."))
}

fn protocol_error(value: &Value) -> QueryError {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let code = value.get("code").and_then(Value::as_i64);
    if message.contains("authentication required")
        || message.contains("sign in")
        || message.contains("login")
    {
        QueryError::signed_out("Please sign in to Codex first.")
    } else if code == Some(-32601) || message.contains("method not found") {
        QueryError::unavailable(
            "This Codex version does not expose quota data. Please update Codex.",
        )
    } else {
        QueryError::unavailable("Codex quota is temporarily unavailable.")
    }
}

async fn read_bounded_line<R>(reader: &mut R) -> Result<Option<String>, QueryError>
where
    R: AsyncBufRead + Unpin,
{
    let mut output = Vec::new();
    loop {
        let (take, found_newline) = {
            let available = reader
                .fill_buf()
                .await
                .map_err(|_| QueryError::unavailable("Codex app-server stopped unexpectedly."))?;
            if available.is_empty() {
                if output.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |position| position + 1);
            if output.len().saturating_add(take) > MAX_PROTOCOL_LINE_BYTES {
                return Err(QueryError::unavailable(
                    "Codex returned an oversized response.",
                ));
            }
            output.extend_from_slice(&available[..take]);
            (take, newline.is_some())
        };
        reader.consume(take);
        if found_newline {
            break;
        }
    }
    if output.last() == Some(&b'\n') {
        output.pop();
    }
    if output.last() == Some(&b'\r') {
        output.pop();
    }
    String::from_utf8(output)
        .map(Some)
        .map_err(|_| QueryError::unavailable("Codex returned malformed protocol data."))
}

async fn read_response<R>(reader: &mut R, request_id: i64) -> Result<Value, QueryError>
where
    R: AsyncBufRead + Unpin,
{
    for _ in 0..256 {
        let line = read_bounded_line(reader)
            .await?
            .ok_or_else(|| QueryError::unavailable("Codex app-server closed unexpectedly."))?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("id").and_then(Value::as_i64) != Some(request_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(protocol_error(error));
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| QueryError::unavailable("Codex returned an incomplete response."));
    }
    Err(QueryError::unavailable(
        "Codex returned too many unrelated messages.",
    ))
}

async fn query_rate_limits() -> Result<Value, QueryError> {
    let mut child = spawn_app_server().await?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| QueryError::unavailable("Codex input stream is unavailable."))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| QueryError::unavailable("Codex output stream is unavailable."))?;
    let mut reader = tokio::io::BufReader::new(stdout);

    write_message(
        &mut stdin,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": {
                    "name": "codex-usage-widget",
                    "title": "Codex Usage",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": false,
                    "optOutNotificationMethods": ["remoteControl/status/changed"]
                }
            }
        }),
    )
    .await?;
    timeout(INITIALIZE_TIMEOUT, read_response(&mut reader, 1))
        .await
        .map_err(|_| QueryError::unavailable("Codex app-server took too long to start."))??;

    write_message(&mut stdin, json!({ "method": "initialized" })).await?;
    write_message(
        &mut stdin,
        json!({ "method": "account/rateLimits/read", "id": 2 }),
    )
    .await?;
    let result = timeout(RATE_LIMIT_TIMEOUT, read_response(&mut reader, 2))
        .await
        .map_err(|_| QueryError::unavailable("Codex quota request timed out."))??;

    drop(stdin);
    let _ = child.start_kill();
    let _ = child.wait().await;
    Ok(result)
}

fn selected_limits(response: &LimitsResponse) -> &LimitSnapshot {
    response
        .rate_limits_by_limit_id
        .as_ref()
        .and_then(|limits| {
            limits.get("codex").or_else(|| {
                limits
                    .values()
                    .find(|snapshot| snapshot.limit_id.as_deref() == Some("codex"))
            })
        })
        .unwrap_or(&response.rate_limits)
}

fn window_by_duration(limits: &LimitSnapshot, duration_mins: i64) -> Option<&RateWindow> {
    [&limits.primary, &limits.secondary]
        .into_iter()
        .flatten()
        .find(|window| window.window_duration_mins == Some(duration_mins))
}

fn usage_window(window: &RateWindow) -> UsageWindow {
    UsageWindow {
        remaining_percent: f64::from((100 - window.used_percent).clamp(0, 100)),
        resets_at: window
            .resets_at
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| time.to_rfc3339()),
        window_seconds: window
            .window_duration_mins
            .and_then(|minutes| u64::try_from(minutes).ok())
            .and_then(|minutes| minutes.checked_mul(60))
            .unwrap_or(0),
    }
}

fn map_response(value: Value) -> Result<ProviderSnapshot, QueryError> {
    let response: LimitsResponse = serde_json::from_value(value)
        .map_err(|_| QueryError::unavailable("Codex quota response format has changed."))?;
    let limits = selected_limits(&response).clone();
    let short = window_by_duration(&limits, 300).or_else(|| {
        limits
            .primary
            .as_ref()
            .filter(|window| window.window_duration_mins.is_none())
    });
    let weekly = window_by_duration(&limits, 10_080).or_else(|| {
        limits
            .secondary
            .as_ref()
            .filter(|window| window.window_duration_mins.is_none())
    });
    if short.is_none() && weekly.is_none() {
        return Err(QueryError::unavailable(
            "Codex did not return a quota window.",
        ));
    }
    let reset_credits = response
        .rate_limit_reset_credits
        .as_ref()
        .and_then(|credits| u64::try_from(credits.available_count).ok());
    let mut reset_credit_expires_at = response
        .rate_limit_reset_credits
        .and_then(|credits| credits.credits)
        .unwrap_or_default()
        .into_iter()
        .filter(|credit| {
            credit
                .status
                .as_deref()
                .is_none_or(|status| status == "available")
        })
        .filter_map(|credit| credit.expires_at)
        .filter_map(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|time| time.to_rfc3339())
        .collect::<Vec<_>>();
    reset_credit_expires_at.sort();
    reset_credit_expires_at.dedup();

    Ok(ProviderSnapshot {
        provider: "codex".into(),
        display_name: "CODEX".into(),
        plan: limits.plan_type.as_ref().map(|value| value.to_uppercase()),
        short_window: short.map(usage_window),
        weekly_window: weekly.map(usage_window),
        reset_credits,
        reset_credit_expires_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: "ok".into(),
        message: None,
    })
}

pub async fn fetch_snapshot() -> ProviderSnapshot {
    match query_rate_limits().await.and_then(map_response) {
        Ok(snapshot) => snapshot,
        Err(error) => ProviderSnapshot::failure(error.status, error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_response() -> Value {
        json!({
            "rateLimits": {
                "limitId": "legacy",
                "primary": { "usedPercent": 90, "windowDurationMins": 60, "resetsAt": 1_893_456_000 },
                "secondary": null,
                "planType": "free"
            },
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "primary": { "usedPercent": 12, "windowDurationMins": 300, "resetsAt": 1_893_456_000 },
                    "secondary": { "usedPercent": 34, "windowDurationMins": 10080, "resetsAt": 1_893_888_000 },
                    "planType": "plus"
                }
            },
            "rateLimitResetCredits": {
                "availableCount": 2,
                "credits": [
                    { "expiresAt": 1_893_456_100, "status": "available" },
                    { "expiresAt": 1_893_456_200, "status": "redeemed" }
                ]
            }
        })
    }

    #[test]
    fn prefers_codex_bucket_and_maps_remaining_percent() {
        let snapshot = map_response(synthetic_response()).expect("synthetic response should map");
        assert_eq!(snapshot.plan.as_deref(), Some("PLUS"));
        assert_eq!(snapshot.short_window.unwrap().remaining_percent, 88.0);
        assert_eq!(snapshot.weekly_window.unwrap().remaining_percent, 66.0);
    }

    #[test]
    fn maps_duration_and_available_reset_credits() {
        let snapshot = map_response(synthetic_response()).expect("synthetic response should map");
        assert_eq!(snapshot.short_window.unwrap().window_seconds, 18_000);
        assert_eq!(snapshot.weekly_window.unwrap().window_seconds, 604_800);
        assert_eq!(snapshot.reset_credits, Some(2));
        assert_eq!(snapshot.reset_credit_expires_at.len(), 1);
    }

    #[test]
    fn maps_weekly_window_when_primary_is_missing() {
        let value = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": null,
                "secondary": {
                    "usedPercent": 34,
                    "windowDurationMins": 10080,
                    "resetsAt": 1_893_888_000
                },
                "planType": "plus"
            },
            "rateLimitsByLimitId": null,
            "rateLimitResetCredits": null
        });
        let snapshot = map_response(value).expect("weekly-only response should map");
        assert_eq!(snapshot.status, "ok");
        assert!(snapshot.short_window.is_none());
        let weekly = snapshot
            .weekly_window
            .expect("weekly window should remain available");
        assert_eq!(weekly.remaining_percent, 66.0);
        assert_eq!(weekly.window_seconds, 604_800);
    }

    #[test]
    fn maps_weekly_window_in_primary_without_using_it_as_short_window() {
        let value = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": {
                    "usedPercent": 34,
                    "windowDurationMins": 10080,
                    "resetsAt": 1_893_888_000
                },
                "secondary": null,
                "planType": "plus"
            },
            "rateLimitsByLimitId": null,
            "rateLimitResetCredits": null
        });
        let snapshot = map_response(value).expect("weekly-only primary response should map");
        assert_eq!(snapshot.status, "ok");
        assert!(snapshot.short_window.is_none());
        let weekly = snapshot
            .weekly_window
            .expect("weekly window should be matched by duration");
        assert_eq!(weekly.remaining_percent, 66.0);
        assert_eq!(weekly.window_seconds, 604_800);
    }

    #[test]
    fn rejects_when_all_quota_windows_are_missing() {
        let value = json!({
            "rateLimits": { "limitId": "codex", "planType": "plus" },
            "rateLimitsByLimitId": null,
            "rateLimitResetCredits": null
        });
        let error = map_response(value).expect_err("missing quota windows should fail");
        assert_eq!(error.status, "unavailable");
        assert_eq!(error.message, "Codex did not return a quota window.");
    }

    #[tokio::test]
    #[ignore = "requires an installed and signed-in Codex"]
    async fn reads_live_rate_limit_shape_without_logging_values() {
        let result = query_rate_limits()
            .await
            .expect("signed-in Codex should return a rate-limit snapshot");
        assert!(result.get("rateLimits").is_some());
        map_response(result).expect("live response should map to the sanitized UI model");
    }
}
