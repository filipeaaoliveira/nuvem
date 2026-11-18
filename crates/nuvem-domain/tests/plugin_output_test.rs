use nuvem_domain::gcp::plugin_output::{OutputStream, PLUGIN_OUTPUT_MAX_BYTES, PluginOutput};

/// Enough identical lines to overrun the bound, each one byte-countable.
fn lines_totalling(bytes: usize) -> String {
    let line = "x".repeat(63); // 63 + newline = 64 bytes per line
    let count = bytes.div_ceil(64);
    std::iter::repeat_n(line, count)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn output_past_the_bound_is_kept_from_the_head_and_reports_what_it_dropped() {
    let output = PluginOutput::capture(&lines_totalling(PLUGIN_OUTPUT_MAX_BYTES * 2), "");

    assert!(output.is_truncated());
    let retained: usize = output.lines().iter().map(|line| line.text.len() + 1).sum();
    assert!(
        retained <= PLUGIN_OUTPUT_MAX_BYTES,
        "retained {retained} bytes, bound is {PLUGIN_OUTPUT_MAX_BYTES}",
    );
    // Head, not tail: `describe` names the resource in its first lines.
    assert_eq!(
        output.lines().first().map(|line| line.text.as_str()),
        Some(&"x".repeat(63)[..])
    );
    assert!(output.dropped_bytes() >= PLUGIN_OUTPUT_MAX_BYTES);
}

#[test]
fn multi_line_stdout_keeps_one_line_per_line() {
    let output = PluginOutput::capture("name: web-01\nstatus: RUNNING\nzone: europe-west1-b", "");

    let texts: Vec<&str> = output
        .lines()
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec!["name: web-01", "status: RUNNING", "zone: europe-west1-b"],
    );
    assert!(
        output
            .lines()
            .iter()
            .all(|line| line.stream == OutputStream::Stdout)
    );
}

#[test]
fn truncation_is_contiguous_so_the_head_has_no_holes_in_it() {
    // A line too big for the budget, then a line that would still fit. Keeping
    // the second would show the operator a head with a gap in it and no way to
    // tell where the gap was.
    let oversized = "x".repeat(PLUGIN_OUTPUT_MAX_BYTES + 1);
    let output = PluginOutput::capture(&format!("{oversized}\nshort tail line"), "");

    assert!(output.is_truncated());
    assert!(
        !output
            .lines()
            .iter()
            .any(|line| line.text == "short tail line"),
        "a line after the cut was admitted: {:?}",
        output.lines(),
    );
}

#[test]
fn a_single_line_longer_than_the_bound_is_cut_on_a_char_boundary() {
    // `gcloud --format=json` prints one line. Dropping it whole would show an
    // empty viewer for the most common machine-readable output there is, and
    // cutting it by bytes would split a multi-byte char.
    let one_line = "é".repeat(PLUGIN_OUTPUT_MAX_BYTES); // 2 bytes each
    let output = PluginOutput::capture(&one_line, "");

    assert!(output.is_truncated());
    let first = output
        .lines()
        .first()
        .expect("the head of the line is kept");
    assert!(first.text.starts_with("éé"), "kept the head of the line");
    assert!(first.text.len() <= PLUGIN_OUTPUT_MAX_BYTES);
    assert!(first.text.chars().all(|c| c == 'é'), "no split char");
}

#[test]
fn stderr_is_kept_after_stdout_and_marked_as_its_own_stream() {
    // A plugin that warns and exits 0 loses the warning today: only stdout is
    // captured at all.
    let output = PluginOutput::capture("name: web-01", "WARNING: quota nearly exhausted");

    let tagged: Vec<(&str, OutputStream)> = output
        .lines()
        .iter()
        .map(|line| (line.text.as_str(), line.stream))
        .collect();
    assert_eq!(
        tagged,
        vec![
            ("name: web-01", OutputStream::Stdout),
            ("WARNING: quota nearly exhausted", OutputStream::Stderr),
        ],
    );
}

#[test]
fn control_characters_never_survive_into_a_line() {
    // A carriage return is whitespace, so the run-preserving rule must not
    // wave it through: a lone `\r` rewrites the line a terminal already drew,
    // and an escape sequence colours output nuvem did not author.
    //
    // The second expectation used to read `"[31mred"`, which is what removing
    // control characters leaves of `ESC [ 3 1 m` — the `ESC` deleted and the
    // rest of the sequence standing there as text. That is the residue this
    // test's own comment says must not happen, written down as if it were the
    // answer. `capture` now strips whole sequences before redaction can
    // destroy the introducer, so all of it goes.
    let output = PluginOutput::capture("status: RUNNING\rerased\n\u{1b}[31mred", "");

    let texts: Vec<&str> = output
        .lines()
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(texts, vec!["status: RUNNING erased", "red"]);
}

#[test]
fn a_credential_is_redacted_on_the_line_it_sits_on() {
    let output = PluginOutput::capture(
        "name: db-password\n  --password=hunter2\n  token: ya29.a0AfH6SMBx",
        "",
    );

    let texts: Vec<&str> = output
        .lines()
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            // Names a credential, does not carry one.
            "name: db-password",
            "  [redacted]",
            "  token: [redacted]",
        ],
    );
}

#[test]
fn indentation_survives_because_yaml_output_is_read_by_its_shape() {
    let output = PluginOutput::capture(
        "networkInterfaces:\n- network: default\n    networkIP: 10.132.0.4",
        "",
    );

    let texts: Vec<&str> = output
        .lines()
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "networkInterfaces:",
            "- network: default",
            "    networkIP: 10.132.0.4",
        ],
    );
}

#[test]
fn the_bound_holds_an_ordinary_describe_whole() {
    // Deliberately a literal rather than `PLUGIN_OUTPUT_MAX_BYTES / 2`: every
    // other test here scales with the constant, so all of them would still
    // pass if the constant itself were wrong.
    let output = PluginOutput::capture(&lines_totalling(200_000), "");

    assert!(
        !output.is_truncated(),
        "200 KB of output was cut, so the bound is smaller than it claims",
    );
    assert_eq!(output.dropped_bytes(), 0);
}

#[test]
fn each_line_spends_only_its_own_bytes_of_the_budget() {
    // A short line, then one far longer than what a *divided* budget would
    // leave. Both fit the real budget, which spends line by line.
    let short = "x".repeat(63);
    let long = "y".repeat(10_000);
    let output = PluginOutput::capture(&format!("{short}\n{long}"), "");

    assert!(!output.is_truncated());
    assert_eq!(output.lines().len(), 2, "both lines were kept");
}

#[test]
fn a_line_that_exactly_fills_the_budget_is_kept_whole() {
    // Cost is the line plus its newline, so this is the last line that fits.
    let exact = "x".repeat(PLUGIN_OUTPUT_MAX_BYTES - 1);
    let output = PluginOutput::capture(&exact, "");

    assert!(
        !output.is_truncated(),
        "a line that exactly fills the budget was treated as overflowing it",
    );
    assert_eq!(output.lines()[0].text.len(), PLUGIN_OUTPUT_MAX_BYTES - 1);
}

#[test]
fn dropped_bytes_counts_exactly_what_the_cut_removed() {
    // One line 1000 bytes past the bound. Its newline is charged too, so the
    // arithmetic is (line + 1) - kept.
    let oversized = "x".repeat(PLUGIN_OUTPUT_MAX_BYTES + 1000);
    let output = PluginOutput::capture(&oversized, "");

    assert_eq!(output.lines()[0].text.len(), PLUGIN_OUTPUT_MAX_BYTES);
    assert_eq!(output.dropped_bytes(), 1001);
}

#[test]
fn a_cut_landing_inside_a_char_steps_back_to_that_chars_boundary() {
    // One ASCII byte then two-byte chars, so the bound lands *between* the
    // halves of an `é` rather than on a boundary — the case a byte-index
    // slice panics on.
    let straddling = format!("a{}", "é".repeat(PLUGIN_OUTPUT_MAX_BYTES));
    let output = PluginOutput::capture(&straddling, "");

    let kept = &output.lines()[0].text;
    assert_eq!(
        kept.len(),
        PLUGIN_OUTPUT_MAX_BYTES - 1,
        "the cut stepped back exactly one byte, to the char boundary",
    );
    assert!(kept.ends_with('é'), "the last char kept is whole");
}

#[test]
fn dropped_bytes_are_reported_in_units_an_operator_reads() {
    let scaled = |dropped: &str| {
        PluginOutput::capture(
            &format!("{}{}", "x".repeat(PLUGIN_OUTPUT_MAX_BYTES), dropped),
            "",
        )
        .dropped_bytes_display()
    };

    // The three arms, each at a size that only one of them can produce.
    assert_eq!(scaled(&"y".repeat(99)), "100 B");
    assert_eq!(scaled(&"y".repeat(4_000)), "3 KiB");
    assert_eq!(scaled(&"y".repeat(3_000_000)), "2 MiB");
}

#[test]
fn a_warning_on_stderr_survives_a_stdout_that_fills_the_whole_bound() {
    // `gcloud ... --format=json` over the bound is exactly when a quota or
    // deprecation warning matters, and a single shared budget spent stdout
    // first would drop every line of it.
    let flood = lines_totalling(PLUGIN_OUTPUT_MAX_BYTES * 2);
    let output = PluginOutput::capture(&flood, "WARNING: quota nearly exhausted");

    let stderr_lines: Vec<&str> = output
        .lines()
        .iter()
        .filter(|line| line.stream == OutputStream::Stderr)
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(stderr_lines, vec!["WARNING: quota nearly exhausted"]);
}

#[test]
fn stderr_gets_no_more_than_its_reserve_when_it_is_the_flood() {
    // The reserve is a floor for stderr, not a second budget: a plugin that
    // writes megabytes to stderr must not double what the type retains.
    let output = PluginOutput::capture("", &lines_totalling(PLUGIN_OUTPUT_MAX_BYTES * 2));

    let retained: usize = output.lines().iter().map(|line| line.text.len() + 1).sum();
    assert!(
        retained <= PLUGIN_OUTPUT_MAX_BYTES,
        "retained {retained} bytes across both streams, bound is {PLUGIN_OUTPUT_MAX_BYTES}",
    );
    assert!(output.is_truncated());
}

#[test]
fn the_stderr_reserve_is_large_enough_for_a_real_warning_block() {
    // A literal size, for the reason the bound test above is a literal: every
    // other stderr test is small enough to survive a reserve of almost any
    // size, so none of them would notice the constant shrinking.
    let warning = lines_totalling(5_000);
    let output = PluginOutput::capture(&lines_totalling(PLUGIN_OUTPUT_MAX_BYTES * 2), &warning);

    let kept = output
        .lines()
        .iter()
        .filter(|line| line.stream == OutputStream::Stderr)
        .count();
    assert_eq!(
        kept,
        warning.lines().count(),
        "the reserve dropped part of a 5 KB warning block",
    );
}

/// A colour sequence must leave nothing readable behind.
///
/// The order inside `capture` is what this pins. Redaction strips control
/// characters, so it deletes the `ESC` that introduces a sequence and leaves
/// `[31m` sitting in the text as ordinary characters — on screen and, via
/// `to_clipboard_text`, in the operator's paste buffer. Stripping the sequence
/// first means all of it goes or none of it does.
#[test]
fn an_ansi_colour_sequence_leaves_no_readable_residue() {
    let output = PluginOutput::capture("\u{1b}[31mDANGER\u{1b}[0m done", "");

    let text = &output.lines()[0].text;
    assert!(
        !text.contains("[31m") && !text.contains("[0m"),
        "the sequence must not survive as readable text: {text:?}"
    );
    assert!(
        text.contains("DANGER") && text.contains("done"),
        "the words around it must survive: {text:?}"
    );
}

/// The string-terminated families go too, and they are the dangerous ones: an
/// OSC sequence can retitle the operator's terminal window.
#[test]
fn an_osc_window_title_sequence_leaves_no_readable_residue() {
    let output = PluginOutput::capture("\u{1b}]0;pwned\u{7}ok", "");

    let text = &output.lines()[0].text;
    assert!(!text.contains("pwned"), "{text:?}");
    assert!(text.contains("ok"), "{text:?}");
}

/// Escapes on stderr are the likelier case — that is the stream tools colour —
/// and it goes through the same capture.
#[test]
fn an_escape_on_stderr_is_stripped_too() {
    let output = PluginOutput::capture("", "\u{1b}[33mWARNING\u{1b}[0m quota");

    let text = &output.lines()[0].text;
    assert!(!text.contains("[33m"), "{text:?}");
    assert!(text.contains("WARNING"), "{text:?}");
}

/// Stripping must not eat ordinary square brackets: `gcloud` prints them in
/// lists and in `[redacted]` itself.
#[test]
fn ordinary_brackets_are_left_alone() {
    let output = PluginOutput::capture("tags: [http-server, https-server]", "");

    assert_eq!(output.lines()[0].text, "tags: [http-server, https-server]");
}
