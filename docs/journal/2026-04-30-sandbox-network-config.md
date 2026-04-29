# Sandbox Network Config

RARA now mirrors Codex-style workspace-write sandbox network configuration with
`sandbox_workspace_write.network_access`.

The default is enabled. This keeps ordinary sandboxed shell and PTY commands
able to use the network without requiring every tool call to set `allow_net`.
Users can still disable network access explicitly with:

```json
{
  "sandbox_workspace_write": {
    "network_access": false
  }
}
```

Tool-level `allow_net` remains supported as an explicit per-call opt-in when the
global config is disabled. Wrapped commands now carry the effective network
state so bash and PTY session environments can expose
`RARA_SANDBOX_NETWORK_DISABLED=1` when sandbox networking is disabled.
Background task records and PTY session snapshots also expose `network_access`
so list/status/stop surfaces show the effective policy that was used when the
process started.

Validated with:

- `cargo test -p rara-config sandbox_workspace -- --nocapture`
- `cargo test -p rara-sandbox linux_sandbox -- --nocapture`
- `cargo test sandbox_command_env -- --nocapture`
- `cargo test background_call_returns_task_and_status_reads_output -- --nocapture`
- `cargo test background_tasks_can_be_listed_and_stopped_without_count_limit -- --nocapture`
- `cargo test sandboxed_pty_env_falls_back_to_process_path_when_snapshot_path_is_missing -- --nocapture`
- `cargo test pty_sessions_can_be_listed_statused_and_stopped -- --nocapture`
- `cargo check --message-format=short`
