# DeepSeek Implicit Thinking History

DeepSeek thinking-capable models can require prior assistant `reasoning_content` even when RARA does not explicitly send a `thinking: enabled` option.

Implementation notes:

- DeepSeek V4/reasoner request construction now folds legacy assistant history without `reasoning_content` whenever thinking is not explicitly disabled.
- The request still does not force-enable thinking for the default path; it only avoids replaying assistant messages that DeepSeek may reject in implicit thinking mode.
- Explicit `thinking: disabled` keeps the previous raw history behavior.
- A focused regression test covers the default-thinking path with legacy assistant history.
