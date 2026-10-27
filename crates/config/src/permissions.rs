use crate::config::PermissionLevel;

pub const REPOSITORY_PERMISSIONS: &[&str] = &[
    "actions",
    "administration",
    "checks",
    "contents",
    "deployments",
    "environments",
    "issues",
    "metadata",
    "packages",
    "pages",
    "pull_requests",
    "repository_hooks",
    "repository_projects",
    "secret_scanning_alerts",
    "secrets",
    "security_events",
    "statuses",
    "vulnerability_alerts",
    "workflows",
];

pub fn required_permissions(
    service: &str,
    action: &str,
) -> &'static [(&'static str, PermissionLevel)] {
    use PermissionLevel::{Read, Write};
    match (service, action) {
        ("git", "push") => &[("contents", Write)],
        ("github", "createPullRequests") => &[("pull_requests", Write)],
        ("github", "checks") => &[("checks", Read)],
        ("github", "commentOnIssue") => &[("issues", Write)],
        ("github", "mergePullRequest") => &[("contents", Write), ("pull_requests", Write)],
        _ => &[],
    }
}
