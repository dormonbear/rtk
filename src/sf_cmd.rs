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

    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error");
    let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or("");
    let command_name = obj
        .get("commandName")
        .and_then(|v| v.as_str())
        .unwrap_or("");

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

/// Parse `sf org list --json` output, deduplicate orgs across categories,
/// and produce a compact summary with alias, username, status, and truncated orgId.
fn filter_org_list(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;

    let categories = [
        "other",
        "sandboxes",
        "nonScratchOrgs",
        "devHubs",
        "scratchOrgs",
    ];
    let mut seen_ids = std::collections::HashSet::new();
    let mut orgs: Vec<String> = Vec::new();

    let mut sandbox_count = 0usize;
    let mut devhub_count = 0usize;
    let mut scratch_count = 0usize;
    let mut other_count = 0usize;

    for cat in &categories {
        if let Some(arr) = result[*cat].as_array() {
            for org in arr {
                let org_id = org["orgId"].as_str().unwrap_or("?");
                if seen_ids.contains(org_id) {
                    continue;
                }
                seen_ids.insert(org_id.to_string());

                match *cat {
                    "sandboxes" => sandbox_count += 1,
                    "devHubs" => devhub_count += 1,
                    "scratchOrgs" => scratch_count += 1,
                    _ => other_count += 1,
                }

                let alias = org["alias"].as_str().unwrap_or("-");
                let username = org["username"].as_str().unwrap_or("?");
                let status = org["connectedStatus"].as_str().unwrap_or("?");
                let id_short = if org_id.len() > 8 {
                    format!("{}...{}", &org_id[..5], &org_id[org_id.len() - 4..])
                } else {
                    org_id.to_string()
                };

                orgs.push(format!(
                    "  {:<16} {:<35} {:<12} {}",
                    alias, username, status, id_short
                ));
            }
        }
    }

    let total = orgs.len();
    let mut parts = Vec::new();
    if sandbox_count > 0 {
        parts.push(format!("{} sandbox", sandbox_count));
    }
    if devhub_count > 0 {
        parts.push(format!("{} devhub", devhub_count));
    }
    if scratch_count > 0 {
        parts.push(format!("{} scratch", scratch_count));
    }
    if other_count > 0 {
        parts.push(format!("{} other", other_count));
    }

    let summary = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    let mut output = format!("SF: {} orgs{}\n", total, summary);

    for org in orgs.iter().take(MAX_ITEMS) {
        output.push_str(org);
        output.push('\n');
    }
    if total > MAX_ITEMS {
        output.push_str(&format!("  ... +{} more\n", total - MAX_ITEMS));
    }

    Some(output.trim_end().to_string())
}

/// Strip `warnings` array and `status` wrapper from `sf org display --json`.
/// Output `result` as compact JSON string.
fn filter_org_display(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;
    Some(result.to_string())
}

/// Parse `sf data query --json` output, strip `attributes` from each record,
/// show count header with done/partial status, truncate at MAX_QUERY_RECORDS.
fn filter_data_query(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;
    let total_size = result["totalSize"].as_u64().unwrap_or(0);
    let done = result["done"].as_bool().unwrap_or(true);
    let records = result["records"].as_array()?;

    let status = if done { "done" } else { "partial" };
    let mut output = format!("Query: {} records ({})\n", total_size, status);

    let mut cleaned: Vec<Value> = Vec::new();
    for (i, record) in records.iter().enumerate() {
        if i >= MAX_QUERY_RECORDS {
            break;
        }
        let mut r = record.clone();
        if let Some(obj) = r.as_object_mut() {
            obj.remove("attributes");
        }
        cleaned.push(r);
    }

    output.push_str(&serde_json::to_string(&cleaned).unwrap_or_default());

    if records.len() > MAX_QUERY_RECORDS {
        output.push_str(&format!(
            "\n... +{} more",
            records.len() - MAX_QUERY_RECORDS
        ));
    }

    Some(output)
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

    // Success: route to subcommand filters
    let sub1 = args.first().map(|s| s.as_str()).unwrap_or("");
    let sub2 = args.get(1).map(|s| s.as_str()).unwrap_or("");

    let filtered = match (sub1, sub2) {
        ("org", "list") => filter_org_list(&stdout),
        ("org", "display") => filter_org_display(&stdout),
        ("data", "query") => filter_data_query(&stdout),
        _ => None,
    };

    let output_str = filtered.unwrap_or_else(|| stdout.to_string());
    println!("{}", output_str);

    timer.track(
        &format!("sf {}", args.join(" ")),
        &format!("rtk sf {}", args.join(" ")),
        &raw,
        &output_str,
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
        let args = vec!["org".to_string(), "list".to_string(), "--json".to_string()];
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

    #[test]
    fn test_filter_org_list_basic() {
        let json = r#"{"status":0,"result":{"other":[{"orgId":"00DdM000007hGKvUAM","username":"dormonbear@test.com","connectedStatus":"Connected","alias":"playground","accessToken":"00DdM...long_token","instanceUrl":"https://test.my.salesforce.com","loginUrl":"https://login.salesforce.com/","clientId":"PlatformCLI","isDevHub":false,"instanceApiVersion":"66.0","instanceApiVersionLastRetrieved":"3/9/2026","isDefaultDevHubUsername":false,"isDefaultUsername":false,"lastUsed":"2026-03-09T10:48:18.633Z"}],"sandboxes":[{"orgId":"00Dp0000000E0zWEAS","username":"dormon.zhou@ef.cn.staging","connectedStatus":"Connected","alias":"OMNI_Staging","accessToken":"00Dp0...long_token","instanceUrl":"https://english1--stg.sandbox.my.salesforce.com","loginUrl":"https://test.salesforce.com/","clientId":"PlatformCLI","isDevHub":false,"isSandbox":true,"instanceApiVersion":"66.0","instanceApiVersionLastRetrieved":"3/8/2026","tracksSource":false,"isDefaultDevHubUsername":false,"isDefaultUsername":false,"lastUsed":"2026-03-09T10:48:18.630Z"}],"nonScratchOrgs":[{"orgId":"00DdM000007hGKvUAM","username":"dormonbear@test.com","connectedStatus":"Connected","alias":"playground","accessToken":"00DdM...long_token","instanceUrl":"https://test.my.salesforce.com"},{"orgId":"00Dp0000000E0zWEAS","username":"dormon.zhou@ef.cn.staging","connectedStatus":"Connected","alias":"OMNI_Staging","accessToken":"00Dp0...long_token","instanceUrl":"https://english1--stg.sandbox.my.salesforce.com"}],"devHubs":[],"scratchOrgs":[]},"warnings":["some warning"]}"#;
        let result = filter_org_list(json).unwrap();
        assert!(result.contains("2 orgs"));
        assert!(result.contains("playground"));
        assert!(result.contains("OMNI_Staging"));
        assert!(result.contains("Connected"));
        assert_eq!(result.matches("playground").count(), 1);
        assert_eq!(result.matches("OMNI_Staging").count(), 1);
    }

    #[test]
    fn test_filter_org_list_token_savings() {
        let json = r#"{"status":0,"result":{"other":[{"orgId":"00D001","username":"user1@test.com","connectedStatus":"Connected","alias":"org1","accessToken":"token_very_long_string_here_1234567890","instanceUrl":"https://org1.my.salesforce.com","loginUrl":"https://login.salesforce.com/","clientId":"PlatformCLI","isDevHub":false,"instanceApiVersion":"66.0","instanceApiVersionLastRetrieved":"3/9/2026","isDefaultDevHubUsername":false,"isDefaultUsername":false,"lastUsed":"2026-03-09T10:48:18.633Z"},{"orgId":"00D002","username":"user2@test.com","connectedStatus":"Connected","alias":"org2","accessToken":"token_very_long_string_here_1234567890","instanceUrl":"https://org2.my.salesforce.com","loginUrl":"https://login.salesforce.com/","clientId":"PlatformCLI","isDevHub":false,"instanceApiVersion":"66.0","instanceApiVersionLastRetrieved":"3/9/2026","isDefaultDevHubUsername":false,"isDefaultUsername":false,"lastUsed":"2026-03-09T10:48:18.633Z"}],"sandboxes":[],"nonScratchOrgs":[{"orgId":"00D001","username":"user1@test.com","connectedStatus":"Connected","alias":"org1","accessToken":"token_very_long_string_here_1234567890","instanceUrl":"https://org1.my.salesforce.com"},{"orgId":"00D002","username":"user2@test.com","connectedStatus":"Connected","alias":"org2","accessToken":"token_very_long_string_here_1234567890","instanceUrl":"https://org2.my.salesforce.com"}],"devHubs":[],"scratchOrgs":[]},"warnings":[]}"#;
        let result = filter_org_list(json).unwrap();
        // Use byte length for savings: compact JSON has few whitespace-delimited tokens
        // but many characters; byte-level savings better reflect actual LLM token reduction
        let savings = 100.0 - (result.len() as f64 / json.len() as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "Org list filter: expected >=60% savings, got {:.1}%",
            savings
        );
    }

    #[test]
    fn test_filter_org_list_empty() {
        let json = r#"{"status":0,"result":{"other":[],"sandboxes":[],"nonScratchOrgs":[],"devHubs":[],"scratchOrgs":[]},"warnings":[]}"#;
        let result = filter_org_list(json).unwrap();
        assert!(result.contains("0 orgs"));
    }

    #[test]
    fn test_filter_org_list_invalid_json() {
        assert!(filter_org_list("not json").is_none());
    }

    #[test]
    fn test_filter_org_display_strips_warnings() {
        let json = r#"{"status":0,"result":{"id":"00Dp0000000E0zWEAS","apiVersion":"66.0","accessToken":"00Dp0...token","instanceUrl":"https://english1--stg.sandbox.my.salesforce.com","username":"dormon.zhou@ef.cn.staging","clientId":"PlatformCLI","connectedStatus":"Connected","alias":"OMNI_Staging"},"warnings":["This command will expose sensitive information that allows for subsequent activity using your current authenticated session. Sharing this information is equivalent to logging someone in under the current credential, resulting in unintended access and escalation of privilege. For additional information, please review the authorization section of the developer docs."]}"#;
        let result = filter_org_display(json).unwrap();
        assert!(result.contains("OMNI_Staging"));
        assert!(result.contains("00Dp0000000E0zWEAS"));
        assert!(!result.contains("sensitive information"));
        assert!(!result.contains("warnings"));
    }

    #[test]
    fn test_filter_org_display_invalid_json() {
        assert!(filter_org_display("not json").is_none());
    }

    #[test]
    fn test_filter_data_query_basic() {
        let json = r#"{"status":0,"result":{"totalSize":3,"done":true,"records":[{"attributes":{"type":"Account","url":"/services/data/v66.0/sobjects/Account/001xx1"},"Id":"001xx1","Name":"Acme Corp","Industry":"Technology"},{"attributes":{"type":"Account","url":"/services/data/v66.0/sobjects/Account/001xx2"},"Id":"001xx2","Name":"Global Inc","Industry":"Finance"},{"attributes":{"type":"Account","url":"/services/data/v66.0/sobjects/Account/001xx3"},"Id":"001xx3","Name":"Local LLC","Industry":"Retail"}]},"warnings":[]}"#;
        let result = filter_data_query(json).unwrap();
        assert!(result.contains("3 records"));
        assert!(result.contains("done"));
        assert!(result.contains("Acme Corp"));
        assert!(!result.contains("attributes"));
        assert!(!result.contains("/services/data"));
    }

    #[test]
    fn test_filter_data_query_truncates_at_50() {
        let mut records = Vec::new();
        for i in 1..=60 {
            records.push(format!(
                r#"{{"attributes":{{"type":"Account","url":"/x"}},"Id":"001{:03}","Name":"Org{}"}}"#,
                i, i
            ));
        }
        let json = format!(
            r#"{{"status":0,"result":{{"totalSize":60,"done":true,"records":[{}]}},"warnings":[]}}"#,
            records.join(",")
        );
        let result = filter_data_query(&json).unwrap();
        assert!(result.contains("60 records"));
        assert!(result.contains("... +10 more"));
    }

    #[test]
    fn test_filter_data_query_not_done() {
        let json = r#"{"status":0,"result":{"totalSize":2000,"done":false,"nextRecordsUrl":"/services/data/v66.0/query/01gxx-2000","records":[{"attributes":{"type":"Account","url":"/x"},"Id":"001xx1","Name":"Test"}]},"warnings":[]}"#;
        let result = filter_data_query(json).unwrap();
        assert!(result.contains("2000 records"));
        assert!(result.contains("partial"));
    }

    #[test]
    fn test_filter_data_query_invalid_json() {
        assert!(filter_data_query("not json").is_none());
    }
}
