use std::io::IsTerminal;

/// Refuses to start a TUI with nowhere to draw it.
///
/// The refusal carries [`crate::exit::NO_TERMINAL`] so a pipeline or a CI step
/// that ran nuvem without a TTY can tell that apart from the app breaking.
pub fn ensure_interactive_terminal() -> anyhow::Result<()> {
    if !terminal_is_interactive(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    ) {
        return Err(crate::exit::Categorised::new(
            crate::exit::NO_TERMINAL,
            "nuvem requires an interactive terminal",
        )
        .into());
    }
    Ok(())
}

/// Whether both ends the TUI needs are attached to a terminal.
///
/// Pure over the two answers rather than asking the process for them, the same
/// shape and for the same reason as
/// [`crate::services::runtime::interactive::tmux_detected_in`]: it makes every
/// combination reachable from a test, including the asymmetric ones a harness
/// cannot produce for itself — it hands its children two pipes, so "a terminal
/// on stdin but not on stdout" never occurs there, and a predicate tested only
/// through the process's own descriptors cannot tell `&&` from `||`.
///
/// Both are required, not either: the app draws to stdout and reads keys from
/// stdin, so a run missing one of them could be started and not completed.
pub fn terminal_is_interactive(stdin_is_terminal: bool, stdout_is_terminal: bool) -> bool {
    stdin_is_terminal && stdout_is_terminal
}

pub fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All four combinations, because the interesting ones are asymmetric:
    /// `nuvem > file` keeps a terminal on stdin and loses it on stdout, and
    /// `nuvem < /dev/null` does the reverse. Both must refuse.
    #[test]
    fn only_a_terminal_on_both_ends_is_interactive() {
        assert!(terminal_is_interactive(true, true));
        assert!(!terminal_is_interactive(true, false));
        assert!(!terminal_is_interactive(false, true));
        assert!(!terminal_is_interactive(false, false));
    }
}
