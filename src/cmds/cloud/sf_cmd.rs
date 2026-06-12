//! Salesforce CLI (sf) output compression.
//!
//! Filters human-readable output of `sf org list` (dedupe orgs repeated
//! across groups) and `sf project deploy` (strip progress noise, keep
//! failures and summary). Explicit `--json` always passes through
//! untouched — machine-readable output is never modified.

use crate::core::runner::{self, RunOptions};
use crate::core::truncate::CAP_LIST;
use crate::core::utils::resolved_command;
use anyhow::Result;

const MAX_ORG_ROWS: usize = CAP_LIST;

/// True when the caller explicitly requested machine-readable output.
fn has_json_flag(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

pub fn run(args: &[String], verbose: u8) -> Result<i32> {
    // Explicit --json = machine-readable intent: never touch it.
    if has_json_flag(args) {
        let os_args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
        return runner::run_passthrough("sf", &os_args, verbose);
    }

    let sub1 = args.first().map(String::as_str);
    let sub2 = args.get(1).map(String::as_str);

    match (sub1, sub2) {
        (Some("org"), Some("list")) => {
            let mut cmd = resolved_command("sf");
            for arg in args {
                cmd.arg(arg);
            }
            if verbose > 0 {
                eprintln!("Running: sf {}", args.join(" "));
            }
            runner::run_filtered_with_exit(
                cmd,
                "sf",
                &args.join(" "),
                |out, code| {
                    if code == 0 {
                        filter_org_list(out)
                    } else {
                        strip_stack_lines(out)
                    }
                },
                RunOptions::stdout_only().tee("sf"),
            )
        }
        (Some("project"), Some("deploy")) => {
            let mut cmd = resolved_command("sf");
            for arg in args {
                cmd.arg(arg);
            }
            if verbose > 0 {
                eprintln!("Running: sf {}", args.join(" "));
            }
            runner::run_filtered_with_exit(
                cmd,
                "sf",
                &args.join(" "),
                filter_deploy,
                RunOptions::default().tee("sf"),
            )
        }
        _ => {
            let os_args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
            runner::run_passthrough("sf", &os_args, verbose)
        }
    }
}

/// Drop stack-trace noise from sf error output; keep message and hint lines.
/// Output is a line subset of the original — no invented format (Transparency).
fn strip_stack_lines(output: &str) -> String {
    output
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("at ") || t.starts_with("Code: ") || t.starts_with("Stack:"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Dedupe org rows repeated across groups (same username), cap data rows
/// at MAX_ORG_ROWS with a tee recovery hint. Non-data lines (headers,
/// separators) pass through; unparseable input passes through whole.
fn filter_org_list(output: &str) -> String {
    let has_data_rows = output.lines().any(|l| l.contains('@'));
    if !has_data_rows {
        return output.to_string(); // Never Block: nothing we recognize
    }

    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<&str> = Vec::new();
    let mut all_rows: Vec<&str> = Vec::new();

    for line in output.lines() {
        match line.split_whitespace().find(|t| t.contains('@')) {
            Some(username) => {
                if !seen.insert(username.to_string()) {
                    continue; // duplicate of an already-shown org
                }
                if all_rows.len() < MAX_ORG_ROWS {
                    kept.push(line);
                }
                all_rows.push(line);
            }
            None => kept.push(line),
        }
    }

    let mut result = kept.join("\n");
    if all_rows.len() > MAX_ORG_ROWS {
        // Tee the full deduped row list (not the raw table) so the tail
        // offset lands exactly on the first hidden row.
        if let Some(hint) = crate::core::tee::force_tee_tail_hint(
            &all_rows.join("\n"),
            "sf-org-list",
            MAX_ORG_ROWS + 1,
        ) {
            result.push('\n');
            result.push_str(&hint);
        }
    }
    result
}

/// Strip repeated in-progress polling lines from deploy output. On failure
/// every failure-detail line is retained — failures are never compressed.
fn filter_deploy(output: &str, _exit_code: i32) -> String {
    // exit_code currently only documents intent: both paths keep everything
    // except progress noise, so failures lose nothing.
    let final_frame = output.rsplit_once("\x1b[G").map_or(output, |(_, frame)| frame);
    final_frame
        .lines()
        .filter(|l| !l.trim_start().starts_with("Status: In Progress"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn test_has_json_flag() {
        assert!(has_json_flag(&s(&["org", "list", "--json"])));
        assert!(!has_json_flag(&s(&["org", "list"])));
        // not fooled by values containing the substring
        assert!(!has_json_flag(&s(&["data", "query", "--file", "x--json.soql"])));
    }

    #[test]
    fn test_strip_stack_lines_drops_trace_keeps_message() {
        let input = "Error (1): The requested resource does not exist\n    at Object.run (/usr/lib/sf/dist/index.js:123:45)\n    at processTicksAndRejections (node:internal/process/task_queues:95:5)\nCode: NOT_FOUND\nTry this:\n  Check the org alias.";
        let out = strip_stack_lines(input);
        assert!(out.contains("Error (1): The requested resource does not exist"));
        assert!(out.contains("Try this:"));
        assert!(out.contains("Check the org alias."));
        assert!(!out.contains("at Object.run"));
        assert!(!out.contains("processTicksAndRejections"));
        assert!(!out.contains("Code: NOT_FOUND"));
    }

    #[test]
    fn test_strip_stack_lines_passes_clean_output() {
        let input = "Deploy Succeeded.\nElapsed Time: 1m 3s";
        assert_eq!(strip_stack_lines(input), input);
    }

    const ORG_LIST_FIXTURE: &str = " Type    Alias         Username                        Org ID              Status\n ─────── ───────────── ─────────────────────────────── ─────────────────── ─────────\n DevHub  SFDC_Live     omni.admin@example.com          00D000000000001EAA  Connected\n         SFDC_Staging  omni.admin@example.com.stg      00D000000000002EAA  Connected\n         SFOA_Live     omni.admin@example.com.oa       00D000000000003EAA  Connected\n         SFDC_Live     omni.admin@example.com          00D000000000001EAA  Connected\n         SFOA_Staging  omni.admin@example.com.oa.stg   00D000000000004EAA  Connected\n         SFDC_Staging  omni.admin@example.com.stg      00D000000000002EAA  Connected\n";

    #[test]
    fn test_org_list_dedupes_by_username() {
        let out = filter_org_list(ORG_LIST_FIXTURE);
        assert_eq!(out.matches("omni.admin@example.com ").count(), 1, "dup row removed");
        assert_eq!(out.matches("omni.admin@example.com.stg ").count(), 1);
        assert!(out.contains("Type"), "header preserved");
        assert!(out.contains("SFOA_Staging"), "unique rows preserved");
    }

    #[test]
    fn test_org_list_caps_rows_with_hint() {
        let mut input = String::from(" Alias  Username  Org ID  Status\n");
        for i in 0..30 {
            input.push_str(&format!(" org{i}  user{i}@example.com  00D{i:015}EAA  Connected\n"));
        }
        let out = filter_org_list(&input);
        let data_rows = out.lines().filter(|l| l.contains('@')).count();
        assert_eq!(data_rows, MAX_ORG_ROWS, "capped at MAX_ORG_ROWS");
        // Hint emission depends on tee config (RTK_TEE / tee.enabled) and is
        // covered by the tee module's own tests — not asserted here.
    }

    #[test]
    fn test_org_list_unparseable_passthrough() {
        let input = "something unexpected without table";
        assert_eq!(filter_org_list(input), input, "Never Block: fallback to raw");
    }

    #[test]
    fn test_org_list_token_savings() {
        let input_tokens = ORG_LIST_FIXTURE.split_whitespace().count();
        let out = filter_org_list(ORG_LIST_FIXTURE);
        let output_tokens = out.split_whitespace().count();
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(savings >= 20.0, "org list dedup: expected >=20% savings on dup-heavy fixture, got {savings:.1}%");
    }

    const DEPLOY_SUCCESS_FIXTURE: &str = "Deploying v60.0 metadata to user@example.com using the v62.0 SOAP API.\nDeploy ID: 0AfC80000012345KAA\nStatus: In Progress | ─ | 0/34 Components\nStatus: In Progress | / | 5/34 Components\nStatus: In Progress | | | 12/34 Components\nStatus: In Progress | \\ | 20/34 Components\nStatus: In Progress | ─ | 28/34 Components\nStatus: In Progress | / | 33/34 Components\nStatus: Succeeded | ─ | 34/34 Components (Members)\n\nDeploy Succeeded.\nElapsed Time: 1m 23s\n";

    const DEPLOY_FAILURE_FIXTURE: &str = "Deploying v60.0 metadata to user@example.com using the v62.0 SOAP API.\nDeploy ID: 0AfC80000012346KAA\nStatus: In Progress | ─ | 0/34 Components\nStatus: In Progress | / | 20/34 Components\nStatus: Failed | ─ | 33/34 Components\n\nComponent Failures [2]\n Type   Name                Problem\n ────── ─────────────────── ──────────────────────────────────────\n Error  MyClass             Variable does not exist: foo (12:5)\n Error  MyTrigger           Method does not exist: bar() (3:10)\n\nDeploy Failed.\nElapsed Time: 0m 45s\n";

    const DEPLOY_TTY_PROGRESS_FIXTURE: &str = " ────────── Deploying Metadata (dry-run) ──────────\n\n ⣾ Preparing 18ms\n ◼ Waiting for the org to respond\n ◼ Deploying Metadata\n\n Status: ⠁\n Deploy ID: 0AfBU000004cKl00AE\n Elapsed Time: 28ms\n\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\x1b[G\n ────────── Deploying Metadata (dry-run) ──────────\n\n ✔ Preparing 659ms\n ◯ Waiting for the org to respond - Skipped\n ✔ Deploying Metadata 2.35s\n   ▸ Components: 1/1 (100%)\n ◯ Running Tests - Skipped\n ◯ Updating Source Tracking - Skipped\n ✔ Done 0ms\n\n Status: Succeeded\n Deploy ID: 0AfBU000004cKl00AE\n Target Org: user@example.com\n Elapsed Time: 3.01s\n\n\nValidated Source\n┌───────────┬─────────────┐\n│ State     │ Name        │\n├───────────┼─────────────┤\n│ Unchanged │ ACRReminder │\n└───────────┴─────────────┘\n\nDry-run complete.\n";

    #[test]
    fn test_deploy_success_strips_progress() {
        let out = filter_deploy(DEPLOY_SUCCESS_FIXTURE, 0);
        assert!(!out.contains("Status: In Progress"), "progress lines dropped");
        assert!(out.contains("Deploy ID: 0AfC80000012345KAA"));
        assert!(out.contains("Status: Succeeded"), "final status kept");
        assert!(out.contains("Deploy Succeeded."));
        assert!(out.contains("Elapsed Time: 1m 23s"));
    }

    #[test]
    fn test_deploy_failure_keeps_all_failure_detail() {
        let out = filter_deploy(DEPLOY_FAILURE_FIXTURE, 1);
        assert!(!out.contains("Status: In Progress"), "progress still dropped");
        assert!(out.contains("Component Failures [2]"));
        assert!(out.contains("Variable does not exist: foo (12:5)"));
        assert!(out.contains("Method does not exist: bar() (3:10)"));
        assert!(out.contains("Deploy Failed."));
    }

    #[test]
    fn test_deploy_tty_progress_keeps_only_final_frame() {
        let out = filter_deploy(DEPLOY_TTY_PROGRESS_FIXTURE, 0);
        assert!(!out.contains("Preparing 18ms"));
        assert!(!out.contains("\x1b[2K"));
        assert!(out.contains("Status: Succeeded"));
        assert!(out.contains("Validated Source"));
        assert!(out.contains("Dry-run complete."));
    }

    #[test]
    fn test_deploy_token_savings() {
        let input_tokens = DEPLOY_SUCCESS_FIXTURE.split_whitespace().count();
        let out = filter_deploy(DEPLOY_SUCCESS_FIXTURE, 0);
        let output_tokens = out.split_whitespace().count();
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(savings >= 50.0, "deploy success: expected >=50% savings, got {savings:.1}%");
    }
}
