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
        if let Some(filtered) = phpstan_cmd::try_filter_phpstan_json(output) {
            return filtered;
        }
    }
    phpstan_cmd::filter_phpstan_text(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phpstan_auto_routes_json_to_json_filter() {
        // `phpstan: ok` is reachable only via the JSON path; the text path needs
        // an "[OK]"/"no errors" line, which this input does not have.
        let json = r#"{"totals":{"errors":0,"file_errors":0},"files":{},"errors":[]}"#;
        assert_eq!(filter_phpstan_auto(json), "phpstan: ok");
    }

    #[test]
    fn test_phpstan_auto_routes_text_to_text_filter() {
        let text = " [ERROR] Found 3 errors\n";
        assert_eq!(filter_phpstan_auto(text), "PHPStan: [ERROR] Found 3 errors");
    }

    #[test]
    fn test_phpstan_auto_falls_back_to_text_on_malformed_json() {
        // Starts with `{` so the JSON path is tried first, but parsing fails —
        // the text path must still get a chance rather than surfacing the
        // JSON parse error.
        let broken = "{not really json\n [OK] No errors\n";
        assert_eq!(filter_phpstan_auto(broken), "phpstan: ok");
    }

    #[test]
    fn test_wrap_rejects_unknown_tool() {
        let err = run("definitely-not-a-tool", &["echo".to_string()], 0)
            .expect_err("unknown tool must not run the command");
        assert!(err.to_string().contains("unknown tool"), "got: {}", err);
    }

    #[test]
    fn test_wrap_requires_a_command() {
        let err = run("phpunit", &[], 0).expect_err("empty command must be rejected");
        assert!(err.to_string().contains("missing command"), "got: {}", err);
    }
}
