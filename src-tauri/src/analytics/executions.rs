use super::{metrics_with_card, projects::ProjectCatalog, types::*};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const PAGE_SIZE: usize = 5;

pub struct ExecutionResults {
    pub rows: Vec<Execution>,
    pub projects: Vec<FilterOption>,
    pub conversations: Vec<FilterOption>,
    pub page: ExecutionPage,
    pub sessions: Vec<ConversationSummary>,
    pub session_page: ExecutionPage,
}

pub fn query_executions_with_prices(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    titles: &HashMap<String, String>,
    catalog: &ProjectCatalog,
    query: &Query,
    now: i64,
    card: &super::pricing::RateCard,
) -> ExecutionResults {
    let mut grouped: HashMap<&str, Vec<&UsageEvent>> = HashMap::new();
    for e in events {
        grouped.entry(&e.root_turn).or_default().push(e);
    }
    let start = query
        .execution_hours
        .filter(|h| [1, 12, 24, 168, 720].contains(h))
        .map(|h| now - i64::from(h) * 3600);
    let mut candidates: Vec<_> = turns
        .values()
        .filter(|t| {
            !t.is_subagent
                && t.completed.is_some_and(|at| at <= now)
                && grouped.contains_key(t.id.as_str())
        })
        .map(|t| (t, catalog.resolve(&t.thread, &t.project)))
        .filter(|(_, p)| match query.project_scope.as_deref() {
            Some("project") => p.is_some(),
            Some("projectless") => p.is_none(),
            _ => true,
        })
        .collect();
    candidates
        .sort_by(|(a, _), (b, _)| b.completed.cmp(&a.completed).then_with(|| a.id.cmp(&b.id)));
    let projects: BTreeMap<_, _> = candidates
        .iter()
        .filter_map(|(_, p)| p.as_ref().map(|p| (p.id.clone(), p.name.clone())))
        .collect();
    candidates.retain(|(t, p)| {
        start.is_none_or(|s| t.completed.unwrap() >= s)
            && query
                .project
                .as_ref()
                .is_none_or(|id| id.is_empty() || p.as_ref().is_some_and(|p| &p.id == id))
    });
    let title = |t: &Turn| super::conversation_title(&t.thread, titles);
    let mut seen = BTreeSet::new();
    let conversations = candidates
        .iter()
        .filter_map(|(t, _)| {
            seen.insert(&t.thread).then(|| FilterOption {
                id: t.thread.clone(),
                name: title(t),
            })
        })
        .collect();
    candidates.retain(|(t, _)| {
        query
            .thread_id
            .as_ref()
            .is_none_or(|id| id.is_empty() || id == &t.thread)
    });
    // Aggregate before pagination so one thread is always one row, including all
    // matching completed turns and their linked child Agent usage exactly once.
    let mut by_thread: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, (turn, _)) in candidates.iter().enumerate() {
        by_thread.entry(&turn.thread).or_default().push(index);
    }
    let mut sessions: Vec<_> = by_thread
        .values()
        .map(|indices| {
            let (latest, project) = &candidates[indices[0]];
            let rows: Vec<_> = indices
                .iter()
                .flat_map(|i| grouped[candidates[*i].0.id.as_str()].iter().copied())
                .collect();
            let models: BTreeSet<_> = rows
                .iter()
                .map(|e| {
                    if e.model.is_empty() {
                        "未知模型".into()
                    } else {
                        e.model.clone()
                    }
                })
                .collect();
            ConversationSummary {
                thread_id: latest.thread.clone(),
                title: title(latest),
                project: project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未归属项目".into()),
                project_id: project.as_ref().map(|p| p.id.clone()),
                models: models.into_iter().collect(),
                execution_count: indices.len(),
                started: indices
                    .iter()
                    .map(|i| candidates[*i].0.started)
                    .min()
                    .unwrap_or(0),
                last_completed: latest.completed.unwrap(),
                metrics: metrics_with_card(rows.into_iter(), card),
            }
        })
        .collect();
    sessions.sort_by(|a, b| {
        b.last_completed
            .cmp(&a.last_completed)
            .then_with(|| a.thread_id.cmp(&b.thread_id))
    });
    let session_total = sessions.len();
    let session_total_pages = session_total.div_ceil(PAGE_SIZE).max(1);
    let session_page = query.page.unwrap_or(1).clamp(1, session_total_pages);
    let sessions = sessions
        .into_iter()
        .skip((session_page - 1) * PAGE_SIZE)
        .take(PAGE_SIZE)
        .collect();
    let total = candidates.len();
    let total_pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = query.page.unwrap_or(1).clamp(1, total_pages);
    let rows = candidates
        .into_iter()
        .skip((page - 1) * PAGE_SIZE)
        .take(PAGE_SIZE)
        .map(|(t, project)| {
            let events = &grouped[t.id.as_str()];
            let models: BTreeSet<_> = events
                .iter()
                .map(|e| {
                    if e.model.is_empty() {
                        "未知模型".into()
                    } else {
                        e.model.clone()
                    }
                })
                .collect();
            Execution {
                id: t.id.clone(),
                title: if t.title.is_empty() {
                    title(t)
                } else {
                    t.title.clone()
                },
                project: project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未归属项目".into()),
                project_id: project.map(|p| p.id),
                thread_id: t.thread.clone(),
                thread_title: title(t),
                models: models.into_iter().collect(),
                started: t.started,
                completed: t.completed.unwrap(),
                metrics: metrics_with_card(events.iter().copied(), card),
            }
        })
        .collect();
    let mut projects: Vec<_> = projects
        .into_iter()
        .map(|(id, name)| FilterOption { id, name })
        .collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    ExecutionResults {
        rows,
        sessions,
        session_page: ExecutionPage {
            page: session_page,
            page_size: PAGE_SIZE,
            total: session_total,
            total_pages: session_total_pages,
        },
        projects,
        conversations,
        page: ExecutionPage {
            page,
            page_size: PAGE_SIZE,
            total,
            total_pages,
        },
    }
}

#[cfg(test)]
fn query_executions(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    titles: &HashMap<String, String>,
    catalog: &ProjectCatalog,
    query: &Query,
    now: i64,
) -> ExecutionResults {
    query_executions_with_prices(
        events,
        turns,
        titles,
        catalog,
        query,
        now,
        &super::pricing::PricingSnapshot::bundled().card,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<UsageEvent>, HashMap<String, Turn>, ProjectCatalog) {
        let mut events = Vec::new();
        let mut turns = HashMap::new();
        for i in 0..37 {
            let id = format!("turn-{i:02}");
            let thread = format!("thread-{}", i % 3);
            let project = if i % 3 == 0 { "/work/app" } else { "/tmp/solo" };
            turns.insert(
                id.clone(),
                Turn {
                    id: id.clone(),
                    thread: thread.clone(),
                    project: project.into(),
                    completed: Some(100000 - i * 1800),
                    ..Default::default()
                },
            );
            events.push(UsageEvent {
                id: id.clone(),
                at: 100000 - i * 1800,
                turn: id.clone(),
                root_turn: id,
                thread,
                project: project.into(),
                parent: None,
                model: "gpt-5.6-sol".into(),
                service_tier: None,
                request_mode: false,
                is_subagent: false,
                tokens: Tokens {
                    input_tokens: 10,
                    ..Default::default()
                },
                modern: true,
            });
        }
        let catalog = serde_json::from_str(
            r#"{"local-projects":{"p":{"name":"App","rootPaths":["/work/app"]}}}"#,
        )
        .unwrap();
        (events, turns, catalog)
    }
    #[test]
    fn paginates_all_history_beyond_old_30_limit_and_clamps_pages() {
        let (events, turns, catalog) = fixture();
        let titles = HashMap::new();
        let first = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query::default(),
            100000,
        );
        assert_eq!(first.rows.len(), 5);
        assert_eq!(first.page.total, 37);
        assert_eq!(first.page.total_pages, 8);
        let last = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query {
                page: Some(usize::MAX),
                ..Default::default()
            },
            100000,
        );
        assert_eq!(last.page.page, 8);
        assert_eq!(last.rows.len(), 2);
        assert_eq!(last.rows[1].id, "turn-36");
    }
    #[test]
    fn conversation_totals_include_all_turns_before_pagination_and_deduplicate_agents() {
        let (mut events, turns, catalog) = fixture();
        let mut child = events[0].clone();
        child.id = "child-response".into();
        child.thread = "child-agent".into();
        child.is_subagent = true;
        events.push(child);
        let r = query_executions(
            &events,
            &turns,
            &HashMap::new(),
            &catalog,
            &Query::default(),
            100000,
        );
        assert_eq!(r.session_page.total, 3);
        assert_eq!(r.sessions[0].execution_count, 13);
        assert_eq!(r.sessions[0].metrics.total_tokens, 140);
        assert_eq!(r.sessions[0].metrics.main_agent_count, 1);
        assert_eq!(r.sessions[0].metrics.sub_agent_count, 1);
        assert_eq!(
            r.sessions
                .iter()
                .map(|s| s.metrics.total_tokens)
                .sum::<u64>(),
            380
        );
        let filtered = query_executions(
            &events,
            &turns,
            &HashMap::new(),
            &catalog,
            &Query {
                execution_hours: Some(1),
                ..Default::default()
            },
            100000,
        );
        assert!(filtered.sessions.iter().all(|s| s.execution_count == 1));
    }
    #[test]
    fn conversation_pages_use_thread_ids_even_when_names_match() {
        let (events, mut turns, catalog) = fixture();
        for (i, t) in turns.values_mut().enumerate() {
            t.thread = format!("conversation-{}", i % 8);
        }
        let titles = turns
            .values()
            .map(|t| (t.thread.clone(), "重名会话".into()))
            .collect();
        let first = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query::default(),
            100000,
        );
        let second = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query {
                page: Some(2),
                ..Default::default()
            },
            100000,
        );
        assert_eq!(first.session_page.total, 8);
        assert_eq!(first.sessions.len(), 5);
        assert_eq!(second.sessions.len(), 3);
        assert!(first
            .sessions
            .iter()
            .all(|a| second.sessions.iter().all(|b| a.thread_id != b.thread_id)));
    }
    #[test]
    fn filters_completion_time_membership_project_and_conversation_before_paging() {
        let (events, turns, catalog) = fixture();
        let titles = HashMap::from([("thread-0".into(), "项目对话".into())]);
        let q = Query {
            execution_hours: Some(1),
            ..Default::default()
        };
        let r = query_executions(&events, &turns, &titles, &catalog, &q, 100000);
        assert_eq!(r.page.total, 3); // inclusive 1-hour boundary
        let r = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query {
                project_scope: Some("projectless".into()),
                ..q.clone()
            },
            100000,
        );
        assert_eq!(r.page.total, 2);
        assert!(r.projects.is_empty());
        assert!(r.rows.iter().all(|r| r.project_id.is_none()));
        let q = Query {
            project: Some("p".into()),
            thread_id: Some("thread-0".into()),
            ..Default::default()
        };
        let r = query_executions(&events, &turns, &titles, &catalog, &q, 100000);
        assert_eq!(r.page.total, 13);
        assert_eq!(r.conversations[0].name, "项目对话");
        assert!(r.rows.iter().all(|r| r.thread_id == "thread-0"));
        let r = query_executions(
            &events,
            &turns,
            &titles,
            &catalog,
            &Query {
                thread_id: Some("missing".into()),
                ..q
            },
            100000,
        );
        assert_eq!(r.page.total, 0);
        assert_eq!(r.page.page, 1);
        assert!(r.rows.is_empty());
        assert_eq!(r.conversations.len(), 1);
    }
}
