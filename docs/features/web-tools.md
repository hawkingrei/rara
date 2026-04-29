# Web Tools

RARA exposes web access through local tools instead of assuming a provider-native
web search surface is available.

## `web_search`

`web_search` uses the Exa MCP HTTP endpoint as the first implementation:

- endpoint: `https://mcp.exa.ai/mcp`;
- optional API key: `EXA_API_KEY`, passed as the `exaApiKey` query parameter;
- API key transport follows Exa's MCP endpoint shape, but RARA does not store
  the key-bearing URL and redacts sensitive URL query parameters in surfaced
  errors;
- protocol: JSON-RPC `tools/call`;
- MCP tool name: `web_search_exa`;
- accepted response formats: JSON and server-sent events;
- timeout: 25 seconds.

The tool input mirrors opencode's Exa tool shape:

- `query`;
- `num_results`, default `8`, clamped to `1..=20`;
- `livecrawl`, `fallback` or `preferred`, default `fallback`;
- `type`, `auto`, `fast`, or `deep`, default `auto`;
- `context_max_characters`, optional, clamped to `1000..=100000`.

The tool result is normalized to:

- `query`;
- `content`;
- `provider`, currently `exa_mcp`.

## `web_fetch`

`web_fetch` fetches a single HTTP or HTTPS URL with bounded runtime behavior:

- allowed schemes: `http`, `https`;
- blocked literal hosts: `localhost`, private IPs, loopback IPs, link-local
  IPs, documentation IPs, and unspecified IPs;
- default timeout: 30 seconds;
- maximum timeout: 120 seconds;
- default response cap: 5 MiB;
- hard response cap: 10 MiB;
- output formats: `markdown`, `text`, `html`.

The result includes:

- original `url`;
- `final_url` after redirects;
- HTTP `status`;
- `content_type`;
- byte count;
- `truncated`;
- `format`;
- `content`.

The first implementation uses a lightweight built-in HTML-to-text conversion for
`markdown` and `text`. A richer markdown conversion layer can be added later
without changing the tool contract.
