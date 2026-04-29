# Execute Autonomy Prompt

RARA's default execute prompt now carries an explicit autonomy and execution-bias section.

Implementation notes:

- Default execute behavior now assumes local tool use and code changes when the user asks for a software task and does not explicitly request planning, discussion, or a no-edit answer.
- The prompt tells the agent not to stop at a proposed solution when inspection, editing, testing, or verification is the next safe step.
- Local reversible actions such as reading files, editing tracked source files, formatting, and focused tests are framed as safe to perform without extra confirmation.
- User confirmation remains required for material unknowns and destructive, hard-to-reverse, or shared external actions.
- The default prompt test now covers the autonomy section so future prompt edits do not silently remove this Codex/Claude-style execution behavior.
