//! The chat slash-command table: a single source of truth for the
//! command names shown in the discoverability popup (see
//! [`ChatPane::render`](super::components)) and dispatched by
//! [`Ui::command`](super::app). Each entry is a canonical command;
//! aliases (e.g. `/afk` for `/away`, `/exit`/`/q` for `/quit`) are
//! resolved by the dispatcher but deliberately omitted here so the popup
//! lists one row per action.

/// One slash command, as surfaced in the suggestion popup.
pub struct SlashCommand {
    /// Canonical name including the leading `/`.
    pub name: &'static str,
    /// Argument hint (e.g. `[name]`), or `""` when the command takes none.
    pub args: &'static str,
    /// One-line description.
    pub help: &'static str,
}

/// Every command offered in the discoverability popup, in display order.
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/ready",
        args: "",
        help: "mark yourself ready",
    },
    SlashCommand {
        name: "/pause",
        args: "",
        help: "mark yourself paused",
    },
    SlashCommand {
        name: "/away",
        args: "[name]",
        help: "mark yourself (or someone) away",
    },
    SlashCommand {
        name: "/me",
        args: "<action>",
        help: "send an action (e.g. * Baughn waves)",
    },
    SlashCommand {
        name: "/watch",
        args: "",
        help: "commit to the current series (wait for you even when away)",
    },
    SlashCommand {
        name: "/maybe",
        args: "",
        help: "set the current series to maybe (the default)",
    },
    SlashCommand {
        name: "/skip",
        args: "",
        help: "stop watching the current series",
    },
    SlashCommand {
        name: "/ack",
        args: "",
        help: "play past a committed-but-absent user (this file)",
    },
    SlashCommand {
        name: "/summon",
        args: "",
        help: "ping absent friends on IRC",
    },
    SlashCommand {
        name: "/settings",
        args: "",
        help: "open settings",
    },
    SlashCommand {
        name: "/quit",
        args: "",
        help: "quit DessPlay",
    },
];

/// The command's invocation signature: `name` alone, or `name args`
/// when it takes arguments (e.g. `/away [name]`). Used both for the
/// popup's tabulated left column and anywhere a command needs naming.
pub fn signature(cmd: &SlashCommand) -> String {
    if cmd.args.is_empty() {
        cmd.name.to_string()
    } else {
        format!("{} {}", cmd.name, cmd.args)
    }
}

/// Commands whose name starts with the typed first token
/// (case-insensitive). Returns empty when `input` does not start with
/// `/`; a bare `/` matches everything. Keys on the first
/// whitespace-delimited token, so once a full command plus a space is
/// typed the popup collapses to that one command (still a useful arg
/// reminder).
pub fn matching(input: &str) -> Vec<&'static SlashCommand> {
    if !input.starts_with('/') {
        return Vec::new();
    }
    // First token, lowercased, including the leading `/`.
    let token = input
        .split_whitespace()
        .next()
        .unwrap_or("/")
        .to_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(&token))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(input: &str) -> Vec<&'static str> {
        matching(input).iter().map(|c| c.name).collect()
    }

    #[test]
    fn bare_slash_matches_all() {
        assert_eq!(matching("/").len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn prefix_narrows() {
        assert_eq!(names("/sk"), vec!["/skip"]);
        assert_eq!(names("/pa"), vec!["/pause"]);
    }

    #[test]
    fn me_is_offered() {
        assert!(names("/").contains(&"/me"));
        assert_eq!(names("/me"), vec!["/me"]);
    }

    #[test]
    fn full_command_plus_space_collapses_to_one() {
        assert_eq!(names("/skip "), vec!["/skip"]);
        assert_eq!(names("/away nero"), vec!["/away"]);
    }

    #[test]
    fn unknown_prefix_matches_nothing() {
        assert!(names("/xyz").is_empty());
    }

    #[test]
    fn non_slash_matches_nothing() {
        assert!(names("hello").is_empty());
        assert!(names("").is_empty());
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(names("/SK"), vec!["/skip"]);
        assert_eq!(names("/Ready"), vec!["/ready"]);
    }
}
