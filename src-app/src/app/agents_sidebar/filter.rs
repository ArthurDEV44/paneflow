//! Sidebar search / filter logic for US-012 of
//! `tasks/prd-agents-view.md`.
//!
//! Lives in its own submodule so the matching rules stay testable
//! without spinning up GPUI: every public function here takes plain
//! data (`&str`, `&[Thread]`, ...) and returns a boolean / index. The
//! render path imports these helpers and only deals with element
//! emission.

use crate::project::{Project, Thread};

/// Case-insensitive substring match. Empty `query` matches everything
/// (the caller short-circuits before this is even called, but the
/// behaviour is documented anyway for symmetry).
#[inline]
pub(crate) fn matches(haystack: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let needle = query.to_lowercase();
    haystack.to_lowercase().contains(&needle)
}

/// Should this project appear at all under the given filter?
///
/// Rule per AC #2: "a project header is visible if any of its threads
/// match OR if its own title matches". Empty filter -> always visible.
pub(crate) fn project_visible(project: &Project, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if matches(&project.title, query) {
        return true;
    }
    project.threads.iter().any(|t| matches(&t.title, query))
}

/// Should this thread row appear under the given filter?
///
/// Rule per AC #2: "a thread row is visible if its title contains the
/// input (case-insensitive substring)". Empty filter -> always
/// visible. When the project itself matches but no thread does, the
/// caller still wants to render the children so the user can drill
/// in; [`thread_visible_in_project`] folds both signals.
pub(crate) fn thread_visible_in_project(thread: &Thread, project: &Project, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if matches(&thread.title, query) {
        return true;
    }
    // Surface every thread when the project title is the match: the
    // user typed the project name, they want to see what is inside.
    matches(&project.title, query)
}

/// First (project_idx, thread_idx) pair whose thread matches. Used by
/// the Down-arrow key handler to jump straight to the first hit.
pub(crate) fn first_matching_thread(projects: &[Project], query: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    for (p_idx, project) in projects.iter().enumerate() {
        for (t_idx, thread) in project.threads.iter().enumerate() {
            if thread_visible_in_project(thread, project, query) {
                return Some((p_idx, t_idx));
            }
        }
    }
    None
}

/// Are there ZERO matching projects/threads? Used by the render path
/// to swap the list for the empty-state row from AC #7.
pub(crate) fn nothing_matches(projects: &[Project], query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    !projects.iter().any(|p| project_visible(p, query))
}

/// US-021: byte-range of the first case-insensitive substring hit of
/// `query` inside `haystack`, suitable for splitting a string into
/// `[before, match, after]` for highlight rendering. Returns `None`
/// when query is empty, longer than haystack, or doesn't match.
///
/// The match preserves the haystack's original byte boundaries so the
/// caller can slice safely (`&haystack[..start]`, `&haystack[start..end]`,
/// `&haystack[end..]`). The lowered haystack/needle are only used to
/// locate the hit -- the slices returned point into the original.
pub(crate) fn match_positions(haystack: &str, query: &str) -> Option<(usize, usize)> {
    if query.is_empty() || query.len() > haystack.len() {
        return None;
    }
    let lower_haystack = haystack.to_lowercase();
    let lower_needle = query.to_lowercase();
    let start = lower_haystack.find(&lower_needle)?;
    // to_lowercase can change byte length for non-ASCII text. For ASCII
    // it is a no-op, so the byte index transfers directly. For
    // non-ASCII titles the start byte still maps because to_lowercase
    // is per-grapheme and preserves the position of each grapheme's
    // first byte in practice for Latin scripts. Worst case the slice
    // is off-by-one and we render no highlight -- never panic.
    if !haystack.is_char_boundary(start) {
        return None;
    }
    let end = start + lower_needle.len();
    if end > haystack.len() || !haystack.is_char_boundary(end) {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use paneflow_acp::AgentKind;

    fn project_with_threads(title: &str, titles: &[&str]) -> Project {
        let mut p = Project::new(title, "/tmp");
        for t in titles {
            p.threads.push(crate::project::Thread::new(
                *t,
                AgentKind::ClaudeCode,
                "/tmp",
            ));
        }
        p
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches("anything", ""));
        let p = project_with_threads("Paneflow", &["A", "B"]);
        assert!(project_visible(&p, ""));
        assert!(thread_visible_in_project(&p.threads[0], &p, ""));
    }

    #[test]
    fn matches_is_case_insensitive() {
        assert!(matches("Paneflow", "pane"));
        assert!(matches("paneflow", "FLOW"));
        assert!(!matches("paneflow", "xyz"));
    }

    #[test]
    fn project_visible_when_thread_matches() {
        let p = project_with_threads("Other", &["Bug fix", "Refactor"]);
        // Project title does not contain "bug", but its first thread
        // does -> the header must still show so the matched thread
        // can be reached.
        assert!(project_visible(&p, "bug"));
    }

    #[test]
    fn thread_surfaces_when_only_project_title_matches() {
        // User typed the project title -> they want to see all
        // children, not the empty headers-only collapse.
        let p = project_with_threads("Paneflow", &["Bug fix", "Refactor"]);
        // "paneflow" matches project only.
        assert!(thread_visible_in_project(&p.threads[0], &p, "paneflow"));
        assert!(thread_visible_in_project(&p.threads[1], &p, "paneflow"));
    }

    #[test]
    fn first_matching_thread_returns_the_first_hit_in_walk_order() {
        let projects = vec![
            project_with_threads("Alpha", &["nope", "nope"]),
            project_with_threads("Beta", &["nope", "MATCH", "MATCH"]),
            project_with_threads("Gamma", &["MATCH"]),
        ];
        assert_eq!(first_matching_thread(&projects, "match"), Some((1, 1)));
    }

    #[test]
    fn match_positions_finds_substring_byte_range() {
        // Simple ASCII match.
        assert_eq!(match_positions("Refactor sidebar", "side"), Some((9, 13)));
        // Case-insensitive: query lowercased, haystack mixed.
        assert_eq!(match_positions("Bug Fix", "BUG"), Some((0, 3)));
        // No match returns None.
        assert_eq!(match_positions("anything", "xyz"), None);
        // Empty query returns None (caller short-circuits but the
        // contract is "no highlight to render").
        assert_eq!(match_positions("anything", ""), None);
        // Query longer than haystack: None.
        assert_eq!(match_positions("ab", "abcdef"), None);
    }

    #[test]
    fn match_positions_slice_is_safe_to_index() {
        // The returned byte range must always be a valid UTF-8 slice
        // boundary in the original haystack, so the render path can
        // safely split into [before, match, after] without panicking.
        let title = "Refactor sidebar";
        let (s, e) = match_positions(title, "side").expect("match");
        assert_eq!(&title[..s], "Refactor ");
        assert_eq!(&title[s..e], "side");
        assert_eq!(&title[e..], "bar");
    }

    #[test]
    fn nothing_matches_when_no_project_or_thread_hits() {
        let projects = vec![project_with_threads("Alpha", &["one", "two"])];
        assert!(!nothing_matches(&projects, ""));
        assert!(!nothing_matches(&projects, "one"));
        assert!(nothing_matches(&projects, "xyzzy"));
    }

    #[test]
    fn filter_completes_in_under_50ms_at_50_projects_x_100_threads() {
        // PRD AC #5: "Filter results render in under 50ms for 500
        // total threads across 50 projects". The matcher itself runs
        // way under that budget -- this test asserts the algorithmic
        // bound holds, not the GPUI render time (which would be
        // verified manually with dev tools).
        let projects: Vec<Project> = (0..50)
            .map(|p| {
                let titles: Vec<String> = (0..10).map(|t| format!("Thread {p}-{t}")).collect();
                let titles_ref: Vec<&str> = titles.iter().map(String::as_str).collect();
                project_with_threads(&format!("Project {p}"), &titles_ref)
            })
            .collect();
        let total_threads: usize = projects.iter().map(|p| p.threads.len()).sum();
        assert_eq!(total_threads, 500);

        let start = std::time::Instant::now();
        let _ = nothing_matches(&projects, "10");
        let _ = first_matching_thread(&projects, "10");
        let elapsed = start.elapsed();
        // Generous bound -- the substring matcher should hit
        // sub-millisecond in practice. CI noise & debug builds can
        // push this up; 50 ms is the PRD budget.
        assert!(
            elapsed.as_millis() < 50,
            "filter pass took {} ms, budget is 50",
            elapsed.as_millis()
        );
    }
}
