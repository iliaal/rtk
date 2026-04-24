//! PHPUnit output filter.
//!
//! Parses PHPUnit's plain-text runner output and emits a compact summary:
//! aggregate counts from the `Tests: X, Assertions: Y, Failures: Z.` line
//! plus a bounded list of failures with their first two detail lines.
//! Dot-progress lines and headers are stripped entirely.

use super::utils::php_tool_command;
use crate::core::runner;
use anyhow::Result;

const MAX_FAILURES_SHOWN: usize = 10;
const MAX_DETAIL_LINES_PER_FAILURE: usize = 2;

pub fn run(args: &[String], verbose: u8) -> Result<i32> {
    let mut cmd = php_tool_command("phpunit");
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: phpunit {}", args.join(" "));
    }

    runner::run_filtered(
        cmd,
        "phpunit",
        &args.join(" "),
        filter_phpunit_output,
        runner::RunOptions::stdout_only().tee("phpunit"),
    )
}

pub(crate) fn filter_phpunit_output(output: &str) -> String {
    let mut failures: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut in_failures = false;

    for line in output.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("OK (") {
            return format!("PHPUnit: {}", trimmed);
        }

        if trimmed.starts_with("OK, but") {
            return build_success_with_skipped(output);
        }

        if (trimmed.starts_with("There was") || trimmed.starts_with("There were"))
            && (trimmed.contains("failure") || trimmed.contains("error"))
        {
            in_failures = true;
            continue;
        }

        if trimmed == "FAILURES!" || trimmed == "ERRORS!" {
            if !current.is_empty() {
                failures.push(std::mem::take(&mut current));
            }
            in_failures = false;
            continue;
        }

        if in_failures {
            if is_numbered_failure_heading(trimmed) {
                if !current.is_empty() {
                    failures.push(std::mem::take(&mut current));
                }
                current.push(trimmed.to_string());
            } else if !trimmed.is_empty() {
                current.push(trimmed.to_string());
            }
        }
    }

    if !current.is_empty() {
        failures.push(current);
    }

    if failures.is_empty() {
        let (tests, assertions, _, _) = parse_counts(output);
        if tests > 0 {
            return format!("PHPUnit: {} tests, {} assertions", tests, assertions);
        }
        return "PHPUnit: ok".to_string();
    }

    build_phpunit_summary(output, &failures)
}

fn is_numbered_failure_heading(line: &str) -> bool {
    // PHPUnit formats each failure as "N) Class::method"
    let mut chars = line.chars();
    let first_digit = chars.next().is_some_and(|c| c.is_ascii_digit());
    first_digit && line.contains(')')
}

fn build_success_with_skipped(output: &str) -> String {
    let (tests, assertions, _, skipped) = parse_counts(output);
    if skipped > 0 {
        format!(
            "PHPUnit: {} tests, {} assertions, {} skipped",
            tests, assertions, skipped
        )
    } else {
        format!("PHPUnit: {} tests, {} assertions", tests, assertions)
    }
}

fn build_phpunit_summary(output: &str, failures: &[Vec<String>]) -> String {
    let (tests, assertions, failures_count, _skipped) = parse_counts(output);

    let mut result = format!(
        "PHPUnit: {} tests, {} assertions, {} failures\n",
        tests, assertions, failures_count
    );

    for failure_lines in failures.iter().take(MAX_FAILURES_SHOWN) {
        if let Some(first) = failure_lines.first() {
            result.push_str(&format!("\n{}\n", first));
        }
        for detail in failure_lines
            .iter()
            .skip(1)
            .take(MAX_DETAIL_LINES_PER_FAILURE)
        {
            result.push_str(&format!("  {}\n", detail));
        }
    }

    if failures.len() > MAX_FAILURES_SHOWN {
        result.push_str(&format!(
            "\n... +{} more failures\n",
            failures.len() - MAX_FAILURES_SHOWN
        ));
    }

    result.trim().to_string()
}

fn parse_counts(output: &str) -> (usize, usize, usize, usize) {
    let mut tests = 0;
    let mut assertions = 0;
    let mut failures = 0;
    let mut skipped = 0;

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("Tests:") {
            continue;
        }

        for part in trimmed.split(',') {
            let mut it = part.split_whitespace();
            let key = it.next().unwrap_or("");
            let val = it
                .next()
                .unwrap_or("")
                .trim_end_matches('.')
                .parse()
                .unwrap_or(0);

            match key {
                "Tests:" => tests = val,
                "Assertions:" => assertions = val,
                k if k.starts_with("Failures") || k.starts_with("Errors") => failures += val,
                k if k.starts_with("Skipped") => skipped = val,
                _ => {}
            }
        }
    }

    (tests, assertions, failures, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_PHPUNIT_FAILURE: &str = r#"PHPUnit 10.5.0 by Sebastian Bergmann and contributors.

Runtime:       PHP 8.2.27 with Xdebug 3.3.1
Configuration: /var/www/html/phpunit.xml

........................................          40 / 40 (100%)
..................................................  80 / 80 (100%)
.F................................................  100 / 100 (100%)
..........                                         110 / 110 (100%)

Time: 00:01:23.456, Memory: 48.00 MB

There was 1 failure:

1) App\Tests\UserTest::testEmailValidation
Failed asserting that false is true.

#0 /var/www/html/src/User.php:142 (App\User::validate)
#1 /var/www/html/tests/UserTest.php:38 (App\Tests\UserTest::testEmailValidation)

FAILURES!
Tests: 110, Assertions: 340, Failures: 1."#;

    const REAL_PHPUNIT_SUCCESS: &str = r#"PHPUnit 10.5.0 by Sebastian Bergmann and contributors.

Runtime:       PHP 8.2.0

.........                                          9 / 9 (100%)

Time: 00:00:00.234, Memory: 6.00 MB

OK (9 tests, 20 assertions)"#;

    const REAL_PHPUNIT_MULTIPLE_FAILURES: &str = r#"PHPUnit 10.5.0 by Sebastian Bergmann and contributors.

FF.......                                         9 / 9 (100%)

Time: 00:00:00.234, Memory: 6.00 MB

There were 2 failures:

1) UserTest::testEmail
Failed asserting that false is true.

/home/user/tests/UserTest.php:42

2) OrderTest::testTotal
Failed asserting that 42 matches expected 100.

/home/user/tests/OrderTest.php:17

FAILURES!
Tests: 9, Assertions: 15, Failures: 2."#;

    #[test]
    fn test_phpunit_success() {
        let result = filter_phpunit_output(REAL_PHPUNIT_SUCCESS);
        assert!(result.contains("PHPUnit"), "got: {}", result);
        assert!(result.contains("OK (9 tests, 20 assertions)"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_failure_captures_test_name() {
        let result = filter_phpunit_output(REAL_PHPUNIT_FAILURE);
        assert!(
            result.contains("UserTest::testEmailValidation"),
            "got: {}",
            result
        );
        assert!(
            result.contains("Failed asserting that false is true"),
            "got: {}",
            result
        );
    }

    #[test]
    fn test_phpunit_failure_summary_counts() {
        let result = filter_phpunit_output(REAL_PHPUNIT_FAILURE);
        assert!(result.contains("110 tests"), "got: {}", result);
        assert!(result.contains("340 assertions"), "got: {}", result);
        assert!(result.contains("1 failures"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_multiple_failures() {
        let result = filter_phpunit_output(REAL_PHPUNIT_MULTIPLE_FAILURES);
        assert!(result.contains("UserTest::testEmail"), "got: {}", result);
        assert!(result.contains("OrderTest::testTotal"), "got: {}", result);
        assert!(result.contains("2 failures"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_ok_with_skipped() {
        let output = r#"OK, but incomplete, skipped, or risky tests!
Tests: 5, Assertions: 10, Skipped: 2."#;
        let result = filter_phpunit_output(output);
        assert!(result.contains("5 tests"), "got: {}", result);
        assert!(result.contains("2 skipped"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_errors_summary() {
        let output = r#"There was 1 error:

1) FooTest::testBar
RuntimeException: boom

ERRORS!
Tests: 1, Assertions: 0, Errors: 1."#;
        let result = filter_phpunit_output(output);
        assert!(result.contains("FooTest::testBar"), "got: {}", result);
        assert!(result.contains("1 failures"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_failure_truncation() {
        let mut output = String::from("There were 15 failures:\n\n");
        for i in 1..=15 {
            output.push_str(&format!(
                "{}) Suite::test{}\nFailed asserting thing {}.\n\n",
                i, i, i
            ));
        }
        output.push_str("FAILURES!\nTests: 15, Assertions: 15, Failures: 15.\n");

        let result = filter_phpunit_output(&output);
        assert!(result.contains("Suite::test1"), "got: {}", result);
        assert!(result.contains("Suite::test10"), "got: {}", result);
        assert!(!result.contains("Suite::test11"), "got: {}", result);
        assert!(result.contains("+5 more failures"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_empty_ok_fallback() {
        let result = filter_phpunit_output("");
        assert_eq!(result, "PHPUnit: ok");
    }

    #[test]
    fn test_phpunit_only_summary_line() {
        let result = filter_phpunit_output("Tests: 4, Assertions: 4.\n");
        assert!(result.contains("4 tests"), "got: {}", result);
    }

    #[test]
    fn test_phpunit_compression() {
        let raw_len = REAL_PHPUNIT_FAILURE.len();
        let filtered_len = filter_phpunit_output(REAL_PHPUNIT_FAILURE).len();
        assert!(
            filtered_len < raw_len / 2,
            "expected >50% reduction, raw={}, filtered={}",
            raw_len,
            filtered_len
        );
    }
}
