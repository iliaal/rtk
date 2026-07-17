//! Shared command execution skeleton for filter modules.

use anyhow::{Context, Result};
use std::process::Command;

use crate::core::stream::{self, FilterMode, StdinMode, StreamFilter};
use crate::core::tracking;

/// Compose `filtered` with an optional recovery `hint`, cap the total at `raw`
/// (never emit more tokens than the command), print it, and return what was
/// emitted so the caller tracks exactly that.
pub fn emit_guarded(filtered: &str, hint: Option<&str>, raw: &str) -> String {
    let body = match hint {
        Some(h) => format!("{}\n{}", filtered, h),
        None => filtered.to_string(),
    };
    let shown = crate::core::guard::never_worse(raw, &body).to_string();
    println!("{}", shown);
    shown
}

pub fn print_with_hint(
    filtered: &str,
    tee_raw: &str,
    guard_raw: &str,
    tee_label: &str,
    exit_code: i32,
) -> String {
    let hint = crate::core::tee::tee_and_hint(tee_raw, tee_label, exit_code);
    emit_guarded(filtered, hint.as_deref(), guard_raw)
}

#[derive(Default)]
pub struct RunOptions<'a> {
    pub tee_label: Option<&'a str>,
    pub filter_stdout_only: bool,
    pub skip_filter_on_failure: bool,
    pub no_trailing_newline: bool,
    /// Forward rtk's own stdin to the child process. Needed for commands that
    /// can read from a pipe (e.g. `cat file | rtk wc`); without it the child
    /// gets an empty stdin and reports zero.
    pub inherit_stdin: bool,
}

impl<'a> RunOptions<'a> {
    pub fn with_tee(label: &'a str) -> Self {
        Self {
            tee_label: Some(label),
            ..Default::default()
        }
    }

    pub fn stdout_only() -> Self {
        Self {
            filter_stdout_only: true,
            ..Default::default()
        }
    }

    pub fn tee(mut self, label: &'a str) -> Self {
        self.tee_label = Some(label);
        self
    }

    pub fn early_exit_on_failure(mut self) -> Self {
        self.skip_filter_on_failure = true;
        self
    }

    pub fn no_trailing_newline(mut self) -> Self {
        self.no_trailing_newline = true;
        self
    }

    pub fn inherit_stdin(mut self) -> Self {
        self.inherit_stdin = true;
        self
    }
}

pub type CaptureFilter<'a> = Box<dyn Fn(&str) -> String + 'a>;
pub type ExitAwareCaptureFilter<'a> = Box<dyn Fn(&str, i32) -> String + 'a>;

pub enum RunMode<'a> {
    Filtered(CaptureFilter<'a>),
    FilteredWithExit(ExitAwareCaptureFilter<'a>),
    Streamed(Box<dyn StreamFilter + 'a>),
    Passthrough,
}

/// The command failed and produced nothing for the filter to read, but did say
/// why on stderr.
///
/// A filter handed an empty stream renders "clean" out of nothing, so without
/// this the summary becomes a fabricated pass — the most dangerous output rtk
/// can produce. Observed with the tool absent: `rtk mypy .` printed
/// "mypy: No issues found" and `rtk vitest` printed "PASS (0) FAIL (0)".
/// Neither tool ran at all; both filters fall back to `python3 -m mypy` / `npx`,
/// which *do* resolve, so the real failure ("No module named mypy") existed only
/// on stderr — which `filter_stdout_only` captures and then drops.
///
/// Keying on the filter's *input* being empty is what makes this safe. A
/// genuinely failing run (tests red, linter found issues) puts its output on
/// stdout, so it never trips this and still gets filtered, which is the filter's
/// whole job. Only "produced nothing and failed" reaches here, and for that the
/// stderr is the entire answer.
pub(crate) fn is_silent_failure(exit_code: i32, filter_input: &str, stderr: &str) -> bool {
    exit_code != 0 && filter_input.trim().is_empty() && !stderr.trim().is_empty()
}

fn run_captured_filter<F>(
    mut cmd: Command,
    tool_name: &str,
    cmd_label: &str,
    filter_fn: F,
    opts: RunOptions<'_>,
    timer: tracking::TimedExecution,
) -> Result<i32>
where
    F: Fn(&str, i32) -> String,
{
    let stdin_mode = if opts.inherit_stdin {
        StdinMode::Inherit
    } else {
        StdinMode::Null
    };
    let result = stream::run_streaming(&mut cmd, stdin_mode, FilterMode::CaptureOnly)
        .with_context(|| format!("Failed to run {}", tool_name))?;

    let exit_code = result.exit_code;
    let raw = &result.raw;
    let raw_stdout = &result.raw_stdout;

    if opts.skip_filter_on_failure && exit_code != 0 {
        if !result.raw_stdout.trim().is_empty() {
            print!("{}", result.raw_stdout);
        }
        if !result.raw_stderr.trim().is_empty() {
            eprint!("{}", result.raw_stderr);
        }
        timer.track(cmd_label, &format!("rtk {}", cmd_label), raw, raw);
        return Ok(exit_code);
    }

    let text_to_filter = if opts.filter_stdout_only {
        raw_stdout
    } else {
        raw
    };

    if is_silent_failure(exit_code, text_to_filter, &result.raw_stderr) {
        eprint!("{}", result.raw_stderr);
        timer.track(cmd_label, &format!("rtk {}", cmd_label), raw, raw);
        return Ok(exit_code);
    }

    let filtered = filter_fn(text_to_filter, exit_code);

    let raw_for_tracking = if opts.filter_stdout_only {
        raw_stdout
    } else {
        raw
    };

    let shown = if let Some(label) = opts.tee_label {
        print_with_hint(&filtered, raw, raw_for_tracking, label, exit_code)
    } else {
        let guarded = crate::core::guard::never_worse(raw_for_tracking, &filtered).to_string();
        if opts.no_trailing_newline {
            print!("{}", guarded);
        } else {
            println!("{}", guarded);
        }
        guarded
    };

    timer.track(
        cmd_label,
        &format!("rtk {}", cmd_label),
        raw_for_tracking,
        &shown,
    );
    Ok(exit_code)
}

pub fn run(
    mut cmd: Command,
    tool_name: &str,
    args_display: &str,
    mode: RunMode<'_>,
    opts: RunOptions<'_>,
) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    let cmd_label = format!("{} {}", tool_name, args_display);

    match mode {
        RunMode::Filtered(filter_fn) => run_captured_filter(
            cmd,
            tool_name,
            &cmd_label,
            move |text, _| filter_fn(text),
            opts,
            timer,
        ),
        RunMode::FilteredWithExit(filter_fn) => run_captured_filter(
            cmd,
            tool_name,
            &cmd_label,
            move |text, exit_code| filter_fn(text, exit_code),
            opts,
            timer,
        ),
        RunMode::Streamed(filter) => {
            let result =
                stream::run_streaming(&mut cmd, StdinMode::Null, FilterMode::Streaming(filter))
                    .with_context(|| format!("Failed to run {}", tool_name))?;

            if let Some(label) = opts.tee_label {
                if let Some(hint) =
                    crate::core::tee::tee_and_hint(&result.raw, label, result.exit_code)
                {
                    println!("{}", hint);
                }
            }

            timer.track(
                &cmd_label,
                &format!("rtk {}", cmd_label),
                &result.raw,
                &result.filtered,
            );
            Ok(result.exit_code)
        }
        RunMode::Passthrough => {
            let result =
                stream::run_streaming(&mut cmd, StdinMode::Inherit, FilterMode::Passthrough)
                    .with_context(|| format!("Failed to run {}", tool_name))?;

            timer.track_passthrough(&cmd_label, &format!("rtk {} (passthrough)", cmd_label));
            Ok(result.exit_code)
        }
    }
}

pub fn run_filtered<F>(
    cmd: Command,
    tool_name: &str,
    args_display: &str,
    filter_fn: F,
    opts: RunOptions<'_>,
) -> Result<i32>
where
    F: Fn(&str) -> String,
{
    run(
        cmd,
        tool_name,
        args_display,
        RunMode::Filtered(Box::new(filter_fn)),
        opts,
    )
}

pub fn run_filtered_with_exit<F>(
    cmd: Command,
    tool_name: &str,
    args_display: &str,
    filter_fn: F,
    opts: RunOptions<'_>,
) -> Result<i32>
where
    F: Fn(&str, i32) -> String,
{
    run(
        cmd,
        tool_name,
        args_display,
        RunMode::FilteredWithExit(Box::new(filter_fn)),
        opts,
    )
}

pub fn run_passthrough(tool: &str, args: &[std::ffi::OsString], verbose: u8) -> Result<i32> {
    if verbose > 0 {
        eprintln!("{} passthrough: {:?}", tool, args);
    }
    let mut cmd = crate::core::utils::resolved_command(tool)?;
    cmd.args(args);
    let args_str = tracking::args_display(args);
    run(
        cmd,
        tool,
        &args_str,
        RunMode::Passthrough,
        RunOptions::default(),
    )
}

pub fn run_streamed(
    cmd: Command,
    tool_name: &str,
    args_display: &str,
    filter: Box<dyn StreamFilter + '_>,
    opts: RunOptions<'_>,
) -> Result<i32> {
    run(
        cmd,
        tool_name,
        args_display,
        RunMode::Streamed(filter),
        opts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silent_failure_detects_tool_that_never_ran() {
        // `rtk mypy .` with mypy uninstalled: the python3 fallback resolves, so
        // no CommandNotFound fires; the tool's absence exists only as stderr.
        assert!(is_silent_failure(
            1,
            "",
            "/usr/bin/python3: No module named mypy\n"
        ));
        // Whitespace-only stdout is still nothing to filter.
        assert!(is_silent_failure(
            1,
            "  \n\n",
            "npm ERR! could not determine executable\n"
        ));
    }

    #[test]
    fn test_silent_failure_leaves_real_red_runs_to_the_filter() {
        // The case that must NOT trip: tests ran and failed. Non-zero exit is
        // normal here and stdout carries the failures — filtering this is the
        // entire point of the tool.
        assert!(!is_silent_failure(
            1,
            "FAIL src/foo.test.ts\n  expected 1 to be 2\n",
            "stderr noise\n"
        ));
        // A linter reporting findings on stdout, exit 1.
        assert!(!is_silent_failure(1, "foo.py:1: error: bad\n", ""));
    }

    #[test]
    fn test_silent_failure_ignores_success_and_silent_stderr() {
        // Clean run producing nothing: "no issues" is a truthful summary.
        assert!(!is_silent_failure(0, "", "some warning\n"));
        // Failed, empty stdout, but no stderr either: nothing to surface, so
        // let the filter render whatever it renders for an empty stream.
        assert!(!is_silent_failure(1, "", ""));
        assert!(!is_silent_failure(1, "", "   \n"));
    }
}
