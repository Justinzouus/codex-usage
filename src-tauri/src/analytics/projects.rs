use super::types::FilterOption;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

// Only deserialize project metadata. No credentials, messages, or settings are retained.
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct ProjectCatalog {
    #[serde(rename = "projectless-thread-ids")]
    projectless: HashSet<String>,
    #[serde(rename = "thread-project-assignments")]
    assignments: HashMap<String, Assignment>,
    #[serde(rename = "local-projects")]
    projects: HashMap<String, Project>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Assignment {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    name: String,
    #[serde(default)]
    root_paths: Vec<String>,
}

impl ProjectCatalog {
    pub fn read(home: &Path) -> Self {
        fs::read(home.join(".codex-global-state.json"))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn resolve(&self, thread: &str, cwd: &str) -> Option<FilterOption> {
        if self.projectless.contains(thread) {
            return None;
        }
        if let Some(a) = self.assignments.get(thread) {
            return Some(FilterOption {
                id: a.project_id.clone(),
                name: self.name(&a.project_id),
            });
        }
        // Longest path wins, using path components rather than string prefixes.
        let matched = self
            .projects
            .iter()
            .flat_map(|(id, p)| p.root_paths.iter().map(move |root| (id, p, root)))
            .filter(|(_, _, root)| !root.is_empty() && Path::new(cwd).starts_with(root))
            .max_by(|(a, _, root_a), (b, _, root_b)| {
                root_a.len().cmp(&root_b.len()).then_with(|| a.cmp(b))
            });
        if let Some((id, p, _)) = matched {
            return Some(FilterOption {
                id: id.clone(),
                name: if p.name.trim().is_empty() {
                    self.name(id)
                } else {
                    p.name.clone()
                },
            });
        }
        None
    }

    pub fn name(&self, id: &str) -> String {
        self.projects
            .get(id)
            .map(|project| project.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("未命名项目 ({id})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_projectless_overrides_shared_cwd_and_assignments_handle_worktrees() {
        let c: ProjectCatalog = serde_json::from_str(
            r#"{
          "projectless-thread-ids":["solo"],
          "thread-project-assignments":{"worktree":{"projectId":"p"}},
          "local-projects":{"p":{"name":"项目","rootPaths":["/work/app"]}}
        }"#,
        )
        .unwrap();
        assert!(c.resolve("solo", "/work/app").is_none());
        assert_eq!(c.resolve("worktree", "/tmp/worktree").unwrap().id, "p");
        assert_eq!(c.resolve("old", "/work/app/src").unwrap().name, "项目");
        assert!(c.resolve("other", "/work/app-other").is_none());
    }
    #[test]
    fn workspace_root_without_a_project_id_is_projectless() {
        let c: ProjectCatalog = serde_json::from_str(
            r#"{"electron-saved-workspace-roots":["/work/wo-h"],"thread-project-assignments":{"known":{"projectId":"p"}}}"#,
        )
        .unwrap();
        assert!(c.resolve("solo", "/work/wo-h/gen").is_none());
        let known = c.resolve("known", "/tmp/gen").unwrap();
        assert_eq!(known.id, "p");
        assert_eq!(known.name, "未命名项目 (p)");
    }
    #[test]
    fn nested_project_uses_longest_matching_root() {
        let c: ProjectCatalog = serde_json::from_str(
            r#"{"local-projects":{
          "p":{"name":"外层","rootPaths":["/work"]},
          "q":{"name":"内层","rootPaths":["/work/app"]}}}"#,
        )
        .unwrap();
        assert_eq!(c.resolve("t", "/work/app/src").unwrap().id, "q");
    }
}
