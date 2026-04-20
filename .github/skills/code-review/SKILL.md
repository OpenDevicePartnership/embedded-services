---
name: code-review
description: AI-assisted code review that produces a structured summary for human reviewers
---

You are reviewing a pull request in the embedded-services repository. Produce a review designed to help a human reviewer understand and evaluate the changes efficiently.

## Output format

### 1. Summary of changes

Write a concise summary (3–5 sentences) of what the PR does and why.

### 2. Step-by-step review guide

Walk the human reviewer through the changes in logical order — not file-by-file, but grouped by concept. For each step:
- Explain what changed and why it matters
- Call out anything non-obvious or that requires domain knowledge

### 3. Potential issues

Present issues in a table:

| # | Severity | File | Description | Code |
|---|----------|------|-------------|------|
| 1 | 🔴 High | `src/example.rs:42` | Description of the issue | `snippet` |
| 2 | 🟡 Medium | `src/other.rs:10` | Description of the issue | `snippet` |
| 3 | 🟢 Low | `src/lib.rs:5` | Description of the issue | `snippet` |

Severity levels:
- 🔴 **High** — Design flaws, incorrect abstractions, safety violations, data loss risks
- 🟡 **Medium** — Missing error handling, incorrect edge cases, concurrency issues, API misuse
- 🟢 **Low** — Suboptimal patterns, missing docs on public APIs, minor improvements

If there are no issues, say so explicitly.

## Review rules

- Do NOT flag formatting, style, or compilation errors — `cargo fmt`, `cargo clippy`, and CI handle those.
- DO focus on **design**: Is the abstraction correct? Does the change fit the existing architecture? Are there better patterns?
- DO evaluate **concurrency correctness** — especially around Embassy async patterns, `Signal`/`Channel` usage, and mutex interactions.
- DO check that `#[cfg(feature = "...")]` gating is correct when code uses embassy, defmt, or log dependencies.
- DO assess **error handling design** — are errors propagated at the right level? Is the error type appropriate?
- DO consider **no_std constraints** — no heap allocation without `heapless`, no `std` types in non-test code.
