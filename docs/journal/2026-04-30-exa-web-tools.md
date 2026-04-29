# Exa Web Tools

Implemented the first local web-search path using Exa MCP, following opencode's
shape:

- added an Exa MCP JSON-RPC client that accepts JSON or SSE responses;
- added `web_search` with `query`, `num_results`, `livecrawl`, `type`, and
  `context_max_characters`;
- registered `web_search` in the full tool manager;
- split the web tool implementation into focused modules;
- hardened `web_fetch` with URL scheme validation, timeout, byte limits,
  redirect metadata, status metadata, and output format selection;
- blocked localhost and private/link-local literal IPs for `web_fetch`, including
  redirect target validation;
- redacted Exa key-bearing URL query parameters in surfaced errors;
- added tool-result compaction and TUI labels for `web_search`.

Validation:

- `cargo test tools::web -- --nocapture`
- `cargo test tool_result::tests::compacts_web_search_with_preview -- --nocapture`
- `cargo check`

Follow-up:

- replace the current lightweight HTML-to-text path with a higher-fidelity
  markdown conversion layer.
