# rtk sf — Salesforce CLI Token-Optimized Filter

## Overview

Add `rtk sf` command to proxy Salesforce CLI (`sf`) with token-optimized output.
Follows the per-subcommand specialized filter pattern (like `aws_cmd.rs`).

## Command Routing

```
rtk sf <args...>

  org list              → filter_org_list()        ~90% savings
  org display           → filter_org_display()     ~70% savings
  data query            → filter_data_query()      ~60-80% savings
  data get record       → filter_data_get_record() ~50-70% savings
  data create/update/delete record → filter_data_mutate() ~60% savings
  project deploy start  → filter_deploy()          ~80% savings
  project deploy report → filter_deploy()          ~80% savings
  *                     → passthrough
```

## Common Behaviors

- **Auto-inject `--json`**: append if not present in args
- **Error compression**: strip `stack`, `cause`, `warnings` from error JSON; show `message` + `code` + `commandName`
- **Exit code preservation**: propagate sf's exit code
- **Fallback**: if JSON parse fails, output raw

## Filter Strategies

### org list

Deduplicate orgs by orgId across all categories (other, sandboxes, nonScratchOrgs, devHubs, scratchOrgs). Per org: alias, username, connectedStatus, orgId (truncated). Group by category with counts. Truncate at 20 orgs.

### org display

Strip `warnings` array and `status` wrapper. Output `result` as compact JSON.

### data query

Strip `attributes` from each record. Show record count header + done status. Output records as JSON array. Truncate at 50 records.

### data get record

Strip `attributes`, `status`, `warnings`. Output `result` fields only.

### data create/update/delete record

Extract `id` + `success`. Show `ok ✓ <id>` or `✗ <error>`.

### project deploy start/report

Extract: id, status, numberComponentsDeployed/Total/Errors. Show componentFailures (type, fullName, problem). Truncate at 20 errors.

### Error output

Parse error JSON, output: `✗ [commandName] code: message`

## File Structure

- `src/sf_cmd.rs` (~400 lines): all filters + tests
- `main.rs`: add `mod sf_cmd`, `Sf` variant, match arm

## Token Savings Targets

| Command | Savings |
|---------|---------|
| org list | ~95% |
| org display | ~80% |
| data query | ~60% |
| data get record | ~75% |
| data mutate | ~90% |
| deploy | ~95% |
| error | ~92% |
