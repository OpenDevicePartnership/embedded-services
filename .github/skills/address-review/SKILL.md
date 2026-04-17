---
name: address-review
description: Evaluate PR review comments for validity and address confirmed ones
---

You are helping a developer respond to review comments on a pull request in the embedded-services repository.

## Important

Do NOT assume review comments are correct. Reviewers (both human and AI) can be wrong. Independently evaluate each comment against the actual code and context before proposing changes.

## Step 1: Evaluate all comments

Read every review comment and the surrounding code. Present your analysis in a table:

| # | Comment | Location | Valid? | Reasoning | Proposed Fix |
|---|---------|----------|--------|-----------|--------------|
| 1 | Reviewer's comment (summarized) | `src/file.rs:42` ([link]) | ✅ Yes / ❌ No / ⚠️ Partial | Why you agree or disagree, with code snippet | What you would change (or "No change needed") |

Include a short code snippet in the Reasoning or Proposed Fix column to show the relevant context:
```
// current code
let val = buf[i];
// proposed fix
let val = buf.get(i).ok_or(MyError::InvalidParams)?;
```

## Step 2: Ask for confirmation

After presenting the table, ask the developer which comments to address by number. Do not make any changes until confirmed.

## Step 3: Apply confirmed fixes

Implement only the confirmed fixes. For comments marked invalid that the developer still wants addressed, discuss the tradeoff before proceeding.

## Guidelines

- When disagreeing with a comment, explain clearly why — reference specific code, types, traits, or constraints.
- Consider the crate's `no_std` context, strict clippy policy, and feature-gated code when evaluating suggestions.
- If a reviewer suggests using `unwrap()`, `panic!()`, or direct indexing, flag it as invalid — these are denied by clippy config.
- If a comment is about style or formatting, mark it as invalid — CI enforces these automatically.
- Create new fixup commits for each change — do NOT amend existing commits. The author will squash them before merge.
