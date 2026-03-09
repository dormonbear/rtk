//! Salesforce CLI (`sf`) output compression.
//!
//! Auto-injects `--json` flag and filters error output to strip verbose
//! stack traces, cause, and warnings. Passthrough for success output (for now).

use crate::tracking;
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

#[allow(dead_code)]
const MAX_ITEMS: usize = 20;
#[allow(dead_code)]
const MAX_QUERY_RECORDS: usize = 50;

/// If args don't contain "--json", append it; otherwise return as-is.
fn ensure_json_flag(args: &[String]) -> Vec<String> {
    if args.iter().any(|a| a == "--json") {
        args.to_vec()
    } else {
        let mut result = args.to_vec();
        result.push("--json".to_string());
        result
    }
}

/// Parse error JSON, extract message/code/commandName.
/// Output compact error: `✗ [commandName] code: message`. Strip stack/cause/warnings.
fn filter_sf_error(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let obj = v.as_object()?;

    let message = obj.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
    let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or("");
    let command_name = obj.get("commandName").and_then(|v| v.as_str()).unwrap_or("");

    // Build compact single-line error, stripping verbose details
    // Format: ✗ commandName code: first_sentence_of_message
    let short_message = message.split(". ").next().unwrap_or(message);

    match (command_name.is_empty(), code.is_empty()) {
        (true, true) => Some(format!("✗ {}", short_message)),
        (true, false) => Some(format!("✗ {}: {}", code, short_message)),
        (false, true) => Some(format!("✗ [{}] {}", command_name, short_message)),
        (false, false) => Some(format!("✗ [{}] {}: {}", command_name, code, short_message)),
    }
}

/// Execute `sf` with args (auto-inject --json), handle errors with filter_sf_error,
/// passthrough success output for now.
pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let final_args = ensure_json_flag(args);

    let mut cmd = Command::new("sf");
    for arg in &final_args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: sf {}", final_args.join(" "));
    }

    let output = cmd.output().context("Failed to run sf CLI")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let raw = format!("{}\n{}", stdout, stderr);

    if !output.status.success() {
        let filtered = filter_sf_error(&stdout)
            .or_else(|| filter_sf_error(&stderr))
            .unwrap_or_else(|| {
                if stderr.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    stderr.trim().to_string()
                }
            });

        timer.track(
            &format!("sf {}", args.join(" ")),
            &format!("rtk sf {}", args.join(" ")),
            &raw,
            &filtered,
        );

        eprintln!("{}", filtered);
        std::process::exit(output.status.code().unwrap_or(1));
    }

    // Success: passthrough for now
    print!("{}", stdout);

    timer.track(
        &format!("sf {}", args.join(" ")),
        &format!("rtk sf {}", args.join(" ")),
        &raw,
        &stdout,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_ensure_json_flag_adds_when_missing() {
        let args = vec!["org".to_string(), "list".to_string()];
        let result = ensure_json_flag(&args);
        assert!(result.contains(&"--json".to_string()));
    }

    #[test]
    fn test_ensure_json_flag_skips_when_present() {
        let args = vec![
            "org".to_string(),
            "list".to_string(),
            "--json".to_string(),
        ];
        let result = ensure_json_flag(&args);
        assert_eq!(result.iter().filter(|a| *a == "--json").count(), 1);
    }

    #[test]
    fn test_filter_sf_error_basic() {
        let json = r#"{"name":"NoDefaultEnvError","message":"No default environment found. Use -o or --target-org to specify an environment.","exitCode":1,"context":"OrgDisplayCommand","stack":"NoDefaultEnvError: No default...\n    at SfCommandError.from ...\n    at OrgDisplayCommand.catch ...","cause":"undefined","warnings":[],"code":"NoDefaultEnvError","status":1,"commandName":"OrgDisplayCommand"}"#;
        let result = filter_sf_error(json).unwrap();
        assert!(result.contains("OrgDisplayCommand"));
        assert!(result.contains("NoDefaultEnvError"));
        assert!(result.contains("No default environment found"));
        assert!(!result.contains("at SfCommandError.from"));
    }

    #[test]
    fn test_filter_sf_error_token_savings() {
        let json = r#"{"name":"NoDefaultEnvError","message":"No default environment found. Use -o or --target-org to specify an environment.","exitCode":1,"context":"OrgDisplayCommand","stack":"NoDefaultEnvError: No default environment found.\n    at SfCommandError.from (file:///long/path/to/SfCommandError.js:48:16)\n    at OrgDisplayCommand.catch (file:///long/path/to/sfCommand.js:332:47)\n    at OrgDisplayCommand._run (/long/path/command.js:186:29)\n    at async Config.runCommand (/long/path/config.js:456:25)","cause":"undefined","warnings":[],"code":"NoDefaultEnvError","status":1,"commandName":"OrgDisplayCommand"}"#;
        let result = filter_sf_error(json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(json) as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "Error filter: expected >=60% savings, got {:.1}%",
            savings
        );
    }

    #[test]
    fn test_filter_sf_error_invalid_json() {
        assert!(filter_sf_error("not json").is_none());
    }

    #[test]
    fn test_filter_sf_error_missing_fields() {
        let json = r#"{"message": "Something failed"}"#;
        let result = filter_sf_error(json).unwrap();
        assert!(result.contains("Something failed"));
    }
}
