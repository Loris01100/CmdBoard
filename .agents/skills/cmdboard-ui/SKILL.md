---
name: cmdboard-ui
description: Modify CmdBoard ratatui screens, responsive layouts, theme slots and translated UI text. Use for UI work in this repository, including the Stats charts; not for unrelated Rust projects or release work.
---

# CmdBoard UI

Read the repository AGENTS.md and design plan sections 7–10 and 15 before changing a screen. Follow links relative to the repository root, three directories above this skill folder.

Inspect the existing screen, its App state and key mappings, Theme loader, and locale keys relevant to the requested change. Reuse existing widgets and formatting helpers when their behavior fits.

## Implementation

- Keep rendering pure. New interactive state belongs in App; map keys to Command and execute through App::execute.
- Add parser/help/completion only for actions actually exposed on the command line. Preserve existing mode-specific bindings.
- Put SQL and aggregates in storage/, not in render functions. Preserve app filtering and local-day semantics when changing Stats.
- Resolve colors through Theme slots. When adding a slot, update all embedded themes and the loader; consider a fallback for existing user themes. The terminal theme must stay within ANSI colors.
- Use t! for labels and help; update every locale with identical placeholders. Keep the global test language unchanged.

## Rendering checks

Use TestBackend to inspect normal, narrow and short layouts, empty data, and long labels. Check that dates align with cells, future days remain blank, and legends describe the actual values and colors. Inspect styles as well as glyphs for color-only charts: printed text cannot verify color.

Test threshold boundaries when changing time buckets. Check Unicode display width when aligning names; character counts alone do not establish terminal-cell width.

Run the relevant tests, formatting check and clippy. Update the design plan for changed behavior. Report whether validation used a test buffer or the real terminal; do not claim a visual check based only on passing tests.
