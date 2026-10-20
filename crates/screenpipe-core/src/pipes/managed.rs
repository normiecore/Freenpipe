// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Organisation-managed pipes and team skills, and the plan pipe quota.
//!
//! In screenpipe's business tiers an organisation could push pipes and team
//! skills to a member's machine. Those were locked: the member could not
//! edit, enable, disable or delete them, and the team skills were injected
//! into every agent session. Account plans also capped how many pipes were
//! loaded, unloading the rest and stopping any that were running.
//!
//! Freenpipe has no organisations and no plans. Every pipe and skill on disk
//! belongs to the local user, and any number of pipes may load. The pipe
//! runtime, the engine API and the desktop app ask these functions instead of
//! answering the questions themselves, so a personal install gives the same
//! answer everywhere.

/// True when an organisation owns this pipe, which would lock it against
/// local edits, toggles and deletion. Always false: every pipe is the user's.
pub fn is_pipe_managed(_pipe_name: &str) -> bool {
    false
}

/// True when an organisation pushed this skill to the machine. Always false.
pub fn is_team_skill_managed(_skill_name: &str) -> bool {
    false
}

/// Names of the team skills an organisation pushed to this machine, which
/// would be injected into agent sessions. Always empty.
pub fn managed_team_skill_names() -> Vec<String> {
    Vec::new()
}

/// Largest number of pipes allowed to load at once, or `None` for no limit.
/// Always `None`: installing more pipes never unloads or stops any.
pub fn pipe_count_limit() -> Option<usize> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_managed() {
        for name in ["obsidian-sync", "daily-summary", "", "team/shared-skill"] {
            assert!(!is_pipe_managed(name), "pipe {name:?}");
            assert!(!is_team_skill_managed(name), "skill {name:?}");
        }
        assert!(managed_team_skill_names().is_empty());
    }

    #[test]
    fn pipe_count_is_unlimited() {
        assert_eq!(pipe_count_limit(), None);
    }
}
