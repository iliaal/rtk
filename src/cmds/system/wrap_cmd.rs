//! Generic command wrapper: run any command and filter its stdout through a named tool filter.
//!
//! Enables rtk's per-tool output compression for commands invoked through
//! wrappers like `docker exec`, `docker compose exec`, `kubectl exec`, or `ssh`.
//! rtk runs on the host, the actual tool runs inside a container, and stdout
//! flows back through the filter on its way to the user.

use crate::cmds::php::{
    ecs_cmd, php_cmd, phpstan_cmd, phpt_cmd, phpunit_cmd, pint_cmd, test_output,
};
use crate::core::runner;
use crate::core::utils::resolved_command;
use anyhow::{bail, Result};

pub fn run(tool: &str, cmd_args: &[String], verbose: u8) -> Result<i32> {
    if cmd_args.is_empty() {
        bail!("rtk wrap <tool> -- <command>: missing command to wrap");
    }

    let filter: fn(&str) -> String = match tool {
        "phpunit" => phpunit_cmd::filter_phpunit_output,
        "phpstan" => filter_phpstan_auto,
        "pest" | "paratest" => test_output::filter_test_runner_output,
        "ecs" => ecs_cmd::filter_ecs_output,
        "pint" => pint_cmd::filter_pint_json,
        "phpt" => phpt_cmd::filter_phpt_output,
        "php-lint" => php_cmd::filter_php_lint_output,
        other => bail!(
            "rtk wrap: unknown tool '{}'. Supported: phpunit, phpstan, pest, paratest, ecs, pint, phpt, php-lint.",
            other
        ),
    };

    let mut cmd = resolved_command(&cmd_args[0])?;
    for a in &cmd_args[1..] {
        cmd.arg(a);
    }

    if verbose > 0 {
        eprintln!("Running (wrap {}): {}", tool, cmd_args.join(" "));
    }

    let tool_label = format!("wrap:{}", tool);
    let tee_label = format!("wrap-{}", tool);
    runner::run_filtered(
        cmd,
        &tool_label,
        &cmd_args.join(" "),
        filter,
        runner::RunOptions::stdout_only().tee(&tee_label),
    )
}

/// Auto-detect JSON vs text output for PHPStan. When the user runs
/// `docker exec app vendor/bin/phpstan analyse` without `--error-format=json`
/// we still want the text path to produce something readable.
fn filter_phpstan_auto(output: &str) -> String {
    let trimmed = output.trim_start();
    if trimmed.starts_with('{') {
        let candidate = phpstan_cmd::filter_phpstan_json(output);
        // filter_phpstan_json returns the fallback_tail message on parse error;
        // fall through to the text path if JSON parsing didn't stick.
        if !candidate.starts_with("phpstan (JSON parse error)") {
            return candidate;
        }
    }
    phpstan_cmd::filter_phpstan_text(output)
}
