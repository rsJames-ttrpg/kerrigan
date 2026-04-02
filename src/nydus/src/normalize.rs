/// Normalize a repo URL or credential pattern to a canonical form.
///
/// Strips protocol (`https://`, `http://`, `git@`), converts SSH `:` to `/`,
/// strips `.git` suffix, and strips trailing `/`.
///
/// Examples:
/// - `git@github.com:rsJames-ttrpg/kerrigan.git` → `github.com/rsJames-ttrpg/kerrigan`
/// - `https://github.com/rsJames-ttrpg/kerrigan.git` → `github.com/rsJames-ttrpg/kerrigan`
/// - `github.com/rsJames-ttrpg/*` → `github.com/rsJames-ttrpg/*` (patterns pass through)
pub fn normalize_repo_url(url: &str) -> String {
    let mut s = url.to_string();

    // Strip protocol
    for prefix in &["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }

    // Strip git@ and convert : to / (SSH URLs)
    if let Some(rest) = s.strip_prefix("git@") {
        s = rest.replacen(':', "/", 1);
    }

    // Strip .git suffix
    if let Some(rest) = s.strip_suffix(".git") {
        s = rest.to_string();
    }

    // Strip trailing slash
    s = s.trim_end_matches('/').to_string();

    s
}

/// Check if a normalized URL matches a normalized pattern.
/// Patterns support trailing `*` wildcard only.
/// Returns the specificity score (length of pattern without wildcard) for ranking.
pub fn pattern_matches(normalized_url: &str, normalized_pattern: &str) -> Option<usize> {
    if let Some(prefix) = normalized_pattern.strip_suffix('*') {
        if normalized_url.starts_with(prefix) {
            Some(prefix.len())
        } else {
            None
        }
    } else if normalized_url == normalized_pattern {
        Some(normalized_pattern.len())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_https() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan.git"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_ssh() {
        assert_eq!(
            normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_no_git_suffix() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_trailing_slash() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan/"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_pattern_passthrough() {
        assert_eq!(
            normalize_repo_url("github.com/rsJames-ttrpg/*"),
            "github.com/rsJames-ttrpg/*"
        );
    }

    #[test]
    fn test_normalize_http() {
        assert_eq!(
            normalize_repo_url("http://gitlab.example.com/group/repo.git"),
            "gitlab.example.com/group/repo"
        );
    }

    #[test]
    fn test_pattern_matches_exact() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/kerrigan");
        assert_eq!(pattern_matches(&url, &pattern), Some(38));
    }

    #[test]
    fn test_pattern_matches_wildcard() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/*");
        assert!(pattern_matches(&url, &pattern).is_some());
    }

    #[test]
    fn test_pattern_no_match() {
        let url = normalize_repo_url("git@github.com:other-org/repo.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/*");
        assert_eq!(pattern_matches(&url, &pattern), None);
    }

    #[test]
    fn test_more_specific_pattern_scores_higher() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let broad = normalize_repo_url("github.com/rsJames-ttrpg/*");
        let exact = normalize_repo_url("github.com/rsJames-ttrpg/kerrigan");
        let broad_score = pattern_matches(&url, &broad).unwrap();
        let exact_score = pattern_matches(&url, &exact).unwrap();
        assert!(exact_score > broad_score);
    }
}
