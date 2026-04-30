# Structured Plan Approval

RARA plan approval should stay on a structured UI path instead of using natural-language keyword matching.

Review notes:

- Treat `<proposed_plan>` as the model-to-runtime signal that a concrete implementation plan is ready for approval.
- Render pending plan approval as an interaction card with explicit options.
- Resume the existing planning workflow only from the selected TUI option, such as `SelectPendingOption(0)` for implementation and `SelectPendingOption(1)` for continued planning.
- Do not parse ordinary composer text with approval or planning keywords such as `continue planning` or `start implementation`.
- While a plan approval card is pending, ordinary non-command text should be blocked with a notice that asks the user to choose from the card or use the numeric shortcut.

The important boundary is that approval is a control-plane event, not a chat message. Keyword matching is brittle because negation, mixed-language phrasing, copied output, or incidental mentions can accidentally resume the workflow with the wrong decision.

Implementation checklist for future changes:

- Keep approval handling in the TUI event path that owns pending interaction options.
- Avoid adding approval fallbacks to the submit path.
- Test that ordinary text does not approve or continue a pending plan.
- Test that each explicit option maps to exactly one runtime continuation action.
