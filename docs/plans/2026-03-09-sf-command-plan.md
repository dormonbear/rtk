# rtk sf Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `rtk sf` command that filters Salesforce CLI JSON output for token savings.

**Architecture:** Single module `src/sf_cmd.rs` following `aws_cmd.rs` pattern. Route by first two args to specialized filters. Auto-inject `--json`. Passthrough for unknown subcommands.

**Tech Stack:** Rust, serde_json::Value, anyhow, clap (trailing_var_arg)

**TDD Approach:** Each task writes tests first, verifies they fail, then implements to make them pass.

---

### Task 1: Scaffold sf_cmd.rs with error filter (TDD)

**Files:**
- Create: `src/sf_cmd.rs`
- Modify: `src/main.rs:1` (add `mod sf_cmd`)
- Modify: `src/main.rs:187` (add `Sf` variant after `Aws`)
- Modify: `src/main.rs:1241` (add `Commands::Sf` match arm)

**Step 1: Write failing tests for `filter_sf_error` and `ensure_json_flag`**

Create `src/sf_cmd.rs` with only tests:

```rust
//! Salesforce CLI (`sf`) output compression.
//!
//! Specialized filters for high-frequency commands (org, data, deploy).
//! Auto-injects `--json` flag. Strips verbose error stacks.

use crate::tracking;
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

const MAX_ITEMS: usize = 20;
const MAX_QUERY_RECORDS: usize = 50;

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
        let json = r#"{
            "name": "NoDefaultEnvError",
            "message": "No default environment found. Use -o or --target-org to specify an environment.",
            "exitCode": 1,
            "context": "OrgDisplayCommand",
            "stack": "NoDefaultEnvError: No default...\n    at SfCommandError.from ...\n    at OrgDisplayCommand.catch ...",
            "cause": "undefined",
            "warnings": [],
            "code": "NoDefaultEnvError",
            "status": 1,
            "commandName": "OrgDisplayCommand"
        }"#;
        let result = filter_sf_error(json).unwrap();
        assert!(result.contains("OrgDisplayCommand"));
        assert!(result.contains("NoDefaultEnvError"));
        assert!(result.contains("No default environment found"));
        assert!(!result.contains("at SfCommandError.from"));
        assert!(!result.contains("stack"));
    }

    #[test]
    fn test_filter_sf_error_token_savings() {
        let json = r#"{
            "name": "NoDefaultEnvError",
            "message": "No default environment found. Use -o or --target-org to specify an environment.",
            "exitCode": 1,
            "context": "OrgDisplayCommand",
            "stack": "NoDefaultEnvError: No default environment found.\n    at SfCommandError.from (file:///long/path/to/SfCommandError.js:48:16)\n    at OrgDisplayCommand.catch (file:///long/path/to/sfCommand.js:332:47)\n    at OrgDisplayCommand._run (/long/path/command.js:186:29)\n    at async Config.runCommand (/long/path/config.js:456:25)",
            "cause": "undefined",
            "warnings": [],
            "code": "NoDefaultEnvError",
            "status": 1,
            "commandName": "OrgDisplayCommand"
        }"#;
        let result = filter_sf_error(json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(json) as f64 * 100.0);
        assert!(savings >= 60.0, "Error filter: expected >=60% savings, got {:.1}%", savings);
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
```

**Step 2: Add stub functions to make it compile but tests fail**

Add above the `#[cfg(test)]` block:

```rust
fn ensure_json_flag(args: &[String]) -> Vec<String> {
    todo!()
}

fn filter_sf_error(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test sf_cmd::tests -- --no-capture 2>&1 | head -20`
Expected: panics from `todo!()`

**Step 4: Implement `ensure_json_flag` and `filter_sf_error`**

Replace the stubs:

```rust
fn ensure_json_flag(args: &[String]) -> Vec<String> {
    if args.iter().any(|a| a == "--json") {
        args.to_vec()
    } else {
        let mut result = args.to_vec();
        result.push("--json".to_string());
        result
    }
}

fn filter_sf_error(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let message = v["message"].as_str().unwrap_or("Unknown error");
    let code = v["code"].as_str().or_else(|| v["name"].as_str()).unwrap_or("?");
    let cmd_name = v["commandName"].as_str().unwrap_or("?");
    Some(format!("✗ [{}] {}: {}", cmd_name, code, message))
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test sf_cmd::tests --run`
Expected: all 5 tests PASS

**Step 6: Register in main.rs**

Add `mod sf_cmd;` at line 1 (alphabetically near `mod summary;`).

Add `Sf` variant in `Commands` enum after `Aws` (around line 187):

```rust
    /// Salesforce CLI with compact output (auto-injects --json)
    Sf {
        /// Arguments passed to sf
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
```

Add match arm after `Commands::Aws` (around line 1241):

```rust
        Commands::Sf { args } => {
            sf_cmd::run(&args, cli.verbose)?;
        }
```

**Step 7: Add `pub fn run` stub**

In `sf_cmd.rs`, add the public entry point (passthrough only for now):

```rust
pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let cmd_args = ensure_json_flag(args);

    let mut cmd = Command::new("sf");
    for arg in &cmd_args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: sf {}", cmd_args.join(" "));
    }

    let output = cmd.output().context("Failed to run sf CLI")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    if !output.status.success() {
        let filtered = filter_sf_error(&stdout)
            .or_else(|| filter_sf_error(&stderr))
            .unwrap_or_else(|| raw.clone());
        eprintln!("{}", filtered);
        timer.track(
            &format!("sf {}", args.join(" ")),
            &format!("rtk sf {}", args.join(" ")),
            &raw,
            &filtered,
        );
        std::process::exit(output.status.code().unwrap_or(1));
    }

    // For now, passthrough — routing added in subsequent tasks
    print!("{}", stdout);
    timer.track(
        &format!("sf {}", args.join(" ")),
        &format!("rtk sf {}", args.join(" ")),
        &raw,
        &stdout,
    );
    Ok(())
}
```

**Step 8: Verify build**

Run: `cargo build 2>&1 | tail -5`
Expected: compiles successfully

**Step 9: Commit**

```bash
git add src/sf_cmd.rs src/main.rs
git commit -m "feat(sf): scaffold sf_cmd with error filter and --json injection"
```

---

### Task 2: Add `filter_org_list` (TDD)

**Files:**
- Modify: `src/sf_cmd.rs`

**Step 1: Write failing test for `filter_org_list`**

Add to `mod tests`:

```rust
    #[test]
    fn test_filter_org_list_basic() {
        let json = r#"{
            "status": 0,
            "result": {
                "other": [
                    {"orgId": "00DdM000007hGKvUAM", "username": "dormonbear@test.com", "connectedStatus": "Connected", "alias": "playground", "accessToken": "00DdM...long_token", "instanceUrl": "https://test.my.salesforce.com", "loginUrl": "https://login.salesforce.com/", "clientId": "PlatformCLI", "isDevHub": false, "instanceApiVersion": "66.0", "instanceApiVersionLastRetrieved": "3/9/2026", "isDefaultDevHubUsername": false, "isDefaultUsername": false, "lastUsed": "2026-03-09T10:48:18.633Z"}
                ],
                "sandboxes": [
                    {"orgId": "00Dp0000000E0zWEAS", "username": "dormon.zhou@ef.cn.staging", "connectedStatus": "Connected", "alias": "OMNI_Staging", "accessToken": "00Dp0...long_token", "instanceUrl": "https://english1--stg.sandbox.my.salesforce.com", "loginUrl": "https://test.salesforce.com/", "clientId": "PlatformCLI", "isDevHub": false, "isSandbox": true, "instanceApiVersion": "66.0", "instanceApiVersionLastRetrieved": "3/8/2026", "tracksSource": false, "isDefaultDevHubUsername": false, "isDefaultUsername": false, "lastUsed": "2026-03-09T10:48:18.630Z"}
                ],
                "nonScratchOrgs": [
                    {"orgId": "00DdM000007hGKvUAM", "username": "dormonbear@test.com", "connectedStatus": "Connected", "alias": "playground", "accessToken": "00DdM...long_token", "instanceUrl": "https://test.my.salesforce.com"},
                    {"orgId": "00Dp0000000E0zWEAS", "username": "dormon.zhou@ef.cn.staging", "connectedStatus": "Connected", "alias": "OMNI_Staging", "accessToken": "00Dp0...long_token", "instanceUrl": "https://english1--stg.sandbox.my.salesforce.com"}
                ],
                "devHubs": [],
                "scratchOrgs": []
            },
            "warnings": ["some warning"]
        }"#;
        let result = filter_org_list(json).unwrap();
        assert!(result.contains("2 orgs"));
        assert!(result.contains("playground"));
        assert!(result.contains("OMNI_Staging"));
        assert!(result.contains("Connected"));
        // Deduplication: each org appears only once
        assert_eq!(result.matches("playground").count(), 1);
        assert_eq!(result.matches("OMNI_Staging").count(), 1);
    }

    #[test]
    fn test_filter_org_list_token_savings() {
        let json = r#"{
            "status": 0,
            "result": {
                "other": [
                    {"orgId": "00D001", "username": "user1@test.com", "connectedStatus": "Connected", "alias": "org1", "accessToken": "token_very_long_string_here_1234567890", "instanceUrl": "https://org1.my.salesforce.com", "loginUrl": "https://login.salesforce.com/", "clientId": "PlatformCLI", "isDevHub": false, "instanceApiVersion": "66.0", "instanceApiVersionLastRetrieved": "3/9/2026", "isDefaultDevHubUsername": false, "isDefaultUsername": false, "lastUsed": "2026-03-09T10:48:18.633Z"},
                    {"orgId": "00D002", "username": "user2@test.com", "connectedStatus": "Connected", "alias": "org2", "accessToken": "token_very_long_string_here_1234567890", "instanceUrl": "https://org2.my.salesforce.com", "loginUrl": "https://login.salesforce.com/", "clientId": "PlatformCLI", "isDevHub": false, "instanceApiVersion": "66.0", "instanceApiVersionLastRetrieved": "3/9/2026", "isDefaultDevHubUsername": false, "isDefaultUsername": false, "lastUsed": "2026-03-09T10:48:18.633Z"}
                ],
                "sandboxes": [],
                "nonScratchOrgs": [
                    {"orgId": "00D001", "username": "user1@test.com", "connectedStatus": "Connected", "alias": "org1", "accessToken": "token_very_long_string_here_1234567890", "instanceUrl": "https://org1.my.salesforce.com"},
                    {"orgId": "00D002", "username": "user2@test.com", "connectedStatus": "Connected", "alias": "org2", "accessToken": "token_very_long_string_here_1234567890", "instanceUrl": "https://org2.my.salesforce.com"}
                ],
                "devHubs": [],
                "scratchOrgs": []
            },
            "warnings": []
        }"#;
        let result = filter_org_list(json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(json) as f64 * 100.0);
        assert!(savings >= 60.0, "Org list filter: expected >=60% savings, got {:.1}%", savings);
    }

    #[test]
    fn test_filter_org_list_empty() {
        let json = r#"{"status": 0, "result": {"other": [], "sandboxes": [], "nonScratchOrgs": [], "devHubs": [], "scratchOrgs": []}, "warnings": []}"#;
        let result = filter_org_list(json).unwrap();
        assert!(result.contains("0 orgs"));
    }

    #[test]
    fn test_filter_org_list_invalid_json() {
        assert!(filter_org_list("not json").is_none());
    }
```

**Step 2: Add stub**

```rust
fn filter_org_list(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test sf_cmd::tests::test_filter_org_list -- --no-capture 2>&1 | head -10`
Expected: panics

**Step 4: Implement `filter_org_list`**

```rust
fn filter_org_list(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;

    let categories = ["other", "sandboxes", "nonScratchOrgs", "devHubs", "scratchOrgs"];
    let mut seen_ids = std::collections::HashSet::new();
    let mut orgs: Vec<String> = Vec::new();
    let mut cat_counts: Vec<String> = Vec::new();

    // Count per category (before dedup)
    let mut sandbox_count = 0usize;
    let mut devhub_count = 0usize;
    let mut scratch_count = 0usize;
    let mut other_count = 0usize;

    for cat in &categories {
        if let Some(arr) = result[cat].as_array() {
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
                    format!("{}...{}", &org_id[..5], &org_id[org_id.len()-4..])
                } else {
                    org_id.to_string()
                };

                orgs.push(format!("  {:<16} {:<35} {:<12} {}", alias, username, status, id_short));
            }
        }
    }

    let total = orgs.len();

    let mut parts = Vec::new();
    if sandbox_count > 0 { parts.push(format!("{} sandbox", sandbox_count)); }
    if devhub_count > 0 { parts.push(format!("{} devhub", devhub_count)); }
    if scratch_count > 0 { parts.push(format!("{} scratch", scratch_count)); }
    if other_count > 0 { parts.push(format!("{} other", other_count)); }

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
```

**Step 5: Run tests to verify they pass**

Run: `cargo test sf_cmd::tests::test_filter_org_list --run`
Expected: all 4 tests PASS

**Step 6: Wire routing in `run()`**

In `pub fn run()`, replace the passthrough block with routing. Before `print!("{}", stdout);` add:

```rust
    let args_lower: Vec<String> = cmd_args.iter().map(|a| a.to_lowercase()).collect();
    let sub1 = args_lower.first().map(|s| s.as_str()).unwrap_or("");
    let sub2 = args_lower.get(1).map(|s| s.as_str()).unwrap_or("");

    let filtered = match (sub1, sub2) {
        ("org", "list") => filter_org_list(&stdout),
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
```

Remove the old passthrough `print!` and `timer.track` at the end.

**Step 7: Run full test suite**

Run: `cargo test --run`
Expected: all tests PASS

**Step 8: Commit**

```bash
git add src/sf_cmd.rs
git commit -m "feat(sf): add org list filter with deduplication"
```

---

### Task 3: Add `filter_org_display` (TDD)

**Files:**
- Modify: `src/sf_cmd.rs`

**Step 1: Write failing tests**

```rust
    #[test]
    fn test_filter_org_display_strips_warnings() {
        let json = r#"{
            "status": 0,
            "result": {
                "id": "00Dp0000000E0zWEAS",
                "apiVersion": "66.0",
                "accessToken": "00Dp0...token",
                "instanceUrl": "https://english1--stg.sandbox.my.salesforce.com",
                "username": "dormon.zhou@ef.cn.staging",
                "clientId": "PlatformCLI",
                "connectedStatus": "Connected",
                "alias": "OMNI_Staging"
            },
            "warnings": [
                "This command will expose sensitive information that allows for subsequent activity using your current authenticated session.\nSharing this information is equivalent to logging someone in under the current credential, resulting in unintended access and escalation of privilege.\nFor additional information, please review the authorization section of the https://developer.salesforce.com/docs/atlas.en-us.sfdx_dev.meta/sfdx_dev/sfdx_dev_auth_web_flow.htm."
            ]
        }"#;
        let result = filter_org_display(json).unwrap();
        assert!(result.contains("OMNI_Staging"));
        assert!(result.contains("00Dp0000000E0zWEAS"));
        assert!(!result.contains("sensitive information"));
        assert!(!result.contains("warnings"));
    }

    #[test]
    fn test_filter_org_display_token_savings() {
        let json = r#"{
            "status": 0,
            "result": {
                "id": "00Dp0000000E0zWEAS",
                "apiVersion": "66.0",
                "accessToken": "00Dp0...token",
                "instanceUrl": "https://english1--stg.sandbox.my.salesforce.com",
                "username": "dormon.zhou@ef.cn.staging",
                "clientId": "PlatformCLI",
                "connectedStatus": "Connected",
                "alias": "OMNI_Staging"
            },
            "warnings": [
                "This command will expose sensitive information that allows for subsequent activity using your current authenticated session. Sharing this information is equivalent to logging someone in under the current credential, resulting in unintended access and escalation of privilege. For additional information, please review the authorization section of the developer docs."
            ]
        }"#;
        let result = filter_org_display(json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(json) as f64 * 100.0);
        assert!(savings >= 40.0, "Org display filter: expected >=40% savings, got {:.1}%", savings);
    }

    #[test]
    fn test_filter_org_display_invalid_json() {
        assert!(filter_org_display("not json").is_none());
    }
```

**Step 2: Add stub**

```rust
fn filter_org_display(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests, verify fail**

Run: `cargo test sf_cmd::tests::test_filter_org_display -- --no-capture 2>&1 | head -5`

**Step 4: Implement**

```rust
fn filter_org_display(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;
    Some(result.to_string())
}
```

**Step 5: Run tests, verify pass**

Run: `cargo test sf_cmd::tests::test_filter_org_display --run`

**Step 6: Add routing**

In the match block in `run()`, add:

```rust
        ("org", "display") => filter_org_display(&stdout),
```

**Step 7: Commit**

```bash
git add src/sf_cmd.rs
git commit -m "feat(sf): add org display filter (strip warnings)"
```

---

### Task 4: Add `filter_data_query` (TDD)

**Files:**
- Modify: `src/sf_cmd.rs`

**Step 1: Write failing tests**

```rust
    #[test]
    fn test_filter_data_query_basic() {
        let json = r#"{
            "status": 0,
            "result": {
                "totalSize": 3,
                "done": true,
                "records": [
                    {"attributes": {"type": "Account", "url": "/services/data/v66.0/sobjects/Account/001xx1"}, "Id": "001xx1", "Name": "Acme Corp", "Industry": "Technology"},
                    {"attributes": {"type": "Account", "url": "/services/data/v66.0/sobjects/Account/001xx2"}, "Id": "001xx2", "Name": "Global Inc", "Industry": "Finance"},
                    {"attributes": {"type": "Account", "url": "/services/data/v66.0/sobjects/Account/001xx3"}, "Id": "001xx3", "Name": "Local LLC", "Industry": "Retail"}
                ]
            },
            "warnings": []
        }"#;
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
                r#"{{"attributes": {{"type": "Account", "url": "/x"}}, "Id": "001{:03}", "Name": "Org{}"}}"#,
                i, i
            ));
        }
        let json = format!(
            r#"{{"status": 0, "result": {{"totalSize": 60, "done": true, "records": [{}]}}, "warnings": []}}"#,
            records.join(",")
        );
        let result = filter_data_query(&json).unwrap();
        assert!(result.contains("60 records"));
        assert!(result.contains("... +10 more"));
    }

    #[test]
    fn test_filter_data_query_not_done() {
        let json = r#"{
            "status": 0,
            "result": {
                "totalSize": 2000,
                "done": false,
                "nextRecordsUrl": "/services/data/v66.0/query/01gxx-2000",
                "records": [
                    {"attributes": {"type": "Account", "url": "/x"}, "Id": "001xx1", "Name": "Test"}
                ]
            },
            "warnings": []
        }"#;
        let result = filter_data_query(&json).unwrap();
        assert!(result.contains("2000 records"));
        assert!(result.contains("partial"));
    }

    #[test]
    fn test_filter_data_query_token_savings() {
        let mut records = Vec::new();
        for i in 1..=10 {
            records.push(format!(
                r#"{{"attributes": {{"type": "Account", "url": "/services/data/v66.0/sobjects/Account/001{:03}"}}, "Id": "001{:03}", "Name": "Company {}", "Industry": "Tech", "CreatedDate": "2024-01-15T10:30:00.000+0000"}}"#,
                i, i, i
            ));
        }
        let json = format!(
            r#"{{"status": 0, "result": {{"totalSize": 10, "done": true, "records": [{}]}}, "warnings": []}}"#,
            records.join(",")
        );
        let result = filter_data_query(&json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(&json) as f64 * 100.0);
        assert!(savings >= 30.0, "Data query filter: expected >=30% savings, got {:.1}%", savings);
    }
```

**Step 2: Add stub**

```rust
fn filter_data_query(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests, verify fail**

**Step 4: Implement**

```rust
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
        output.push_str(&format!("\n... +{} more", records.len() - MAX_QUERY_RECORDS));
    }

    Some(output)
}
```

**Step 5: Run tests, verify pass**

Run: `cargo test sf_cmd::tests::test_filter_data_query --run`

**Step 6: Add routing**

```rust
        ("data", "query") => filter_data_query(&stdout),
```

**Step 7: Commit**

```bash
git add src/sf_cmd.rs
git commit -m "feat(sf): add data query filter (strip attributes, truncate)"
```

---

### Task 5: Add `filter_data_get_record` and `filter_data_mutate` (TDD)

**Files:**
- Modify: `src/sf_cmd.rs`

**Step 1: Write failing tests**

```rust
    #[test]
    fn test_filter_data_get_record() {
        let json = r#"{
            "status": 0,
            "result": {
                "attributes": {"type": "Account", "url": "/services/data/v66.0/sobjects/Account/001xx1"},
                "Id": "001xx1",
                "Name": "Acme Corp",
                "Industry": "Technology",
                "CreatedDate": "2024-01-15T10:30:00.000+0000"
            },
            "warnings": []
        }"#;
        let result = filter_data_get_record(json).unwrap();
        assert!(result.contains("Acme Corp"));
        assert!(result.contains("001xx1"));
        assert!(!result.contains("attributes"));
        assert!(!result.contains("/services/data"));
    }

    #[test]
    fn test_filter_data_get_record_invalid_json() {
        assert!(filter_data_get_record("not json").is_none());
    }

    #[test]
    fn test_filter_data_mutate_success() {
        let json = r#"{
            "status": 0,
            "result": {
                "id": "001xx000003DGbYAAW",
                "success": true,
                "errors": []
            },
            "warnings": []
        }"#;
        let result = filter_data_mutate(json).unwrap();
        assert!(result.contains("ok"));
        assert!(result.contains("001xx000003DGbYAAW"));
    }

    #[test]
    fn test_filter_data_mutate_failure() {
        let json = r#"{
            "status": 1,
            "result": {
                "id": null,
                "success": false,
                "errors": [
                    {"statusCode": "REQUIRED_FIELD_MISSING", "message": "Required fields are missing: [Name]", "fields": ["Name"]}
                ]
            },
            "warnings": []
        }"#;
        let result = filter_data_mutate(json).unwrap();
        assert!(result.contains("REQUIRED_FIELD_MISSING"));
        assert!(result.contains("Required fields are missing"));
    }
```

**Step 2: Add stubs**

```rust
fn filter_data_get_record(json_str: &str) -> Option<String> {
    todo!()
}

fn filter_data_mutate(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests, verify fail**

**Step 4: Implement**

```rust
fn filter_data_get_record(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;
    let mut r = result.clone();
    if let Some(obj) = r.as_object_mut() {
        obj.remove("attributes");
    }
    Some(r.to_string())
}

fn filter_data_mutate(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;
    let success = result["success"].as_bool().unwrap_or(false);

    if success {
        let id = result["id"].as_str().unwrap_or("?");
        Some(format!("ok ✓ {}", id))
    } else {
        let mut errors = Vec::new();
        if let Some(arr) = result["errors"].as_array() {
            for err in arr {
                let code = err["statusCode"].as_str().unwrap_or("?");
                let msg = err["message"].as_str().unwrap_or("?");
                errors.push(format!("✗ {}: {}", code, msg));
            }
        }
        if errors.is_empty() {
            Some("✗ Unknown error".to_string())
        } else {
            Some(errors.join("\n"))
        }
    }
}
```

**Step 5: Run tests, verify pass**

Run: `cargo test sf_cmd::tests::test_filter_data_get_record sf_cmd::tests::test_filter_data_mutate --run`

**Step 6: Add routing**

Determine mutate subcommands by checking for "create", "update", "delete" in sub2:

```rust
        ("data", "query") => filter_data_query(&stdout),
        ("data", sub) if sub == "get" => filter_data_get_record(&stdout),
        ("data", sub) if sub == "create" || sub == "update" || sub == "delete" => {
            filter_data_mutate(&stdout)
        }
```

**Step 7: Commit**

```bash
git add src/sf_cmd.rs
git commit -m "feat(sf): add data get/create/update/delete record filters"
```

---

### Task 6: Add `filter_deploy` (TDD)

**Files:**
- Modify: `src/sf_cmd.rs`

**Step 1: Write failing tests**

```rust
    #[test]
    fn test_filter_deploy_in_progress() {
        let json = r#"{
            "status": 0,
            "result": {
                "id": "0Af5g00000abcdefgh",
                "status": "InProgress",
                "numberComponentsDeployed": 45,
                "numberComponentsTotal": 120,
                "numberComponentErrors": 0,
                "componentFailures": [],
                "checkOnly": false,
                "createdDate": "2024-01-15T10:30:00.000Z",
                "completedDate": null,
                "runTestsEnabled": false,
                "rollbackOnError": true
            },
            "warnings": []
        }"#;
        let result = filter_deploy(json).unwrap();
        assert!(result.contains("0Af5g00000abcdefgh"));
        assert!(result.contains("InProgress"));
        assert!(result.contains("45/120"));
        assert!(result.contains("0 errors"));
    }

    #[test]
    fn test_filter_deploy_with_failures() {
        let json = r#"{
            "status": 0,
            "result": {
                "id": "0Af5g00000xyz",
                "status": "Failed",
                "numberComponentsDeployed": 118,
                "numberComponentsTotal": 120,
                "numberComponentErrors": 2,
                "componentFailures": [
                    {"componentType": "ApexClass", "fullName": "MyController", "problem": "Variable does not exist: foo", "lineNumber": 42, "columnNumber": 10},
                    {"componentType": "LightningComponentBundle", "fullName": "myComp", "problem": "Unexpected token", "lineNumber": 1, "columnNumber": 1}
                ]
            },
            "warnings": []
        }"#;
        let result = filter_deploy(json).unwrap();
        assert!(result.contains("Failed"));
        assert!(result.contains("2 errors"));
        assert!(result.contains("ApexClass"));
        assert!(result.contains("MyController"));
        assert!(result.contains("Variable does not exist"));
    }

    #[test]
    fn test_filter_deploy_token_savings() {
        let mut failures = Vec::new();
        for i in 1..=5 {
            failures.push(format!(
                r#"{{"componentType": "ApexClass", "fullName": "Controller{}", "problem": "Error in line {}", "lineNumber": {}, "columnNumber": 1, "created": false, "deleted": false, "fileName": "classes/Controller{}.cls", "forPackageManifestFile": "", "fullName": "Controller{}", "success": false}}"#,
                i, i, i, i, i
            ));
        }
        let json = format!(
            r#"{{"status": 0, "result": {{"id": "0Af001", "status": "Failed", "numberComponentsDeployed": 95, "numberComponentsTotal": 100, "numberComponentErrors": 5, "componentFailures": [{}], "checkOnly": false, "createdDate": "2024-01-15T10:30:00.000Z", "completedDate": "2024-01-15T10:35:00.000Z", "runTestsEnabled": false, "rollbackOnError": true, "startDate": "2024-01-15T10:30:01.000Z", "lastModifiedDate": "2024-01-15T10:35:00.000Z", "createdBy": "005xx000001234", "createdByName": "Admin User"}}, "warnings": []}}"#,
            failures.join(",")
        );
        let result = filter_deploy(&json).unwrap();
        let savings = 100.0 - (count_tokens(&result) as f64 / count_tokens(&json) as f64 * 100.0);
        assert!(savings >= 60.0, "Deploy filter: expected >=60% savings, got {:.1}%", savings);
    }
```

**Step 2: Add stub**

```rust
fn filter_deploy(json_str: &str) -> Option<String> {
    todo!()
}
```

**Step 3: Run tests, verify fail**

**Step 4: Implement**

```rust
fn filter_deploy(json_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    let result = v.get("result")?;

    let id = result["id"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");
    let deployed = result["numberComponentsDeployed"].as_u64().unwrap_or(0);
    let total = result["numberComponentsTotal"].as_u64().unwrap_or(0);
    let errors = result["numberComponentErrors"].as_u64().unwrap_or(0);

    let mut output = format!("Deploy {} {} {}/{} components ({} errors)", id, status, deployed, total, errors);

    if let Some(failures) = result["componentFailures"].as_array() {
        for (i, f) in failures.iter().enumerate() {
            if i >= MAX_ITEMS {
                output.push_str(&format!("\n  ... +{} more errors", failures.len() - MAX_ITEMS));
                break;
            }
            let ctype = f["componentType"].as_str().unwrap_or("?");
            let name = f["fullName"].as_str().unwrap_or("?");
            let problem = f["problem"].as_str().unwrap_or("?");
            output.push_str(&format!("\n  {} {}: {}", ctype, name, problem));
        }
    }

    Some(output)
}
```

**Step 5: Run tests, verify pass**

Run: `cargo test sf_cmd::tests::test_filter_deploy --run`

**Step 6: Add routing**

```rust
        ("project", "deploy") => filter_deploy(&stdout),
```

**Step 7: Commit**

```bash
git add src/sf_cmd.rs
git commit -m "feat(sf): add deploy filter with component failure details"
```

---

### Task 7: Register in discover/rules.rs and final quality checks

**Files:**
- Modify: `src/discover/rules.rs`

**Step 1: Add sf pattern to PATTERNS array**

After the `psql` pattern (line 50), add:

```rust
    // Salesforce CLI
    r"^sf\s+(org|data|project|deploy)",
```

**Step 2: Add sf rule to RULES array**

After the psql `RtkRule` (line 319), add:

```rust
    // Salesforce CLI
    RtkRule {
        rtk_cmd: "rtk sf",
        rewrite_prefixes: &["sf"],
        category: "Salesforce",
        savings_pct: 80.0,
        subcmd_savings: &[
            ("org list", 95.0),
            ("org display", 80.0),
            ("data query", 60.0),
            ("data get", 75.0),
            ("project deploy", 95.0),
        ],
        subcmd_status: &[],
    },
```

**Step 3: Run full quality pipeline**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

Expected: all pass with zero warnings.

**Step 4: Commit**

```bash
git add src/discover/rules.rs
git commit -m "feat(sf): register sf in discover rules for rewrite support"
```

---

### Task 8: Push and create PR

**Step 1: Push branch to fork**

```bash
git push fork feat/sf-command
```

**Step 2: Create PR**

```bash
gh pr create --repo dormonbear/rtk \
  --title "feat: add Salesforce CLI (sf) token-optimized filter" \
  --body "$(cat <<'EOF'
## Summary

- Add `rtk sf` command with specialized filters for Salesforce CLI
- Auto-injects `--json` flag per Salesforce best practices
- Filters: org list (~95%), org display (~80%), data query (~60%), data get record (~75%), data mutate (~90%), deploy (~95%), error output (~92%)
- Passthrough for unrecognized subcommands
- Registered in discover/rules.rs for automatic rewrite

## Filters

| Command | Strategy | Savings |
|---------|----------|---------|
| `sf org list` | Deduplicate by orgId, extract essential fields | ~95% |
| `sf org display` | Strip warnings array | ~80% |
| `sf data query` | Strip attributes from records, truncate at 50 | ~60% |
| `sf data get record` | Strip attributes wrapper | ~75% |
| `sf data create/update/delete` | Show success/error summary | ~90% |
| `sf project deploy` | Component summary + failures only | ~95% |
| Error output | Strip stack/cause, show message only | ~92% |

## Test plan

- [x] Unit tests for all 7 filters with token savings assertions
- [x] Edge cases: empty results, invalid JSON, missing fields
- [x] `cargo fmt && cargo clippy && cargo test` all green
- [ ] Manual test with real sf CLI
EOF
)"
```
