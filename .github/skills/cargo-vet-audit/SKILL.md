---
name: cargo-vet-audit
description: >
  Orchestrates cargo-vet supply chain audits for Rust crates. Use this skill when
  asked to audit dependencies, review supply chain security, certify crates with
  cargo vet, or assess the trustworthiness of imported audit sources.
---

# Cargo-Vet Audit Skill

You are orchestrating a `cargo vet` supply chain audit. Follow this process end-to-end.

## Step 1: Discover Unvetted Crates

Run `cargo vet` and parse the output. Omit `--locked` only if `imports.lock` needs
to be refreshed for imported third-party audits; this does not refer to updating
`Cargo.lock`.
Categorize each unvetted crate as either:

- **Delta audit** — a version-to-version diff (e.g., `1.0.0 → 1.1.0`)
- **Full audit** — a complete source inspection of a single version

Note the recommended commands from cargo vet's output (e.g., `cargo vet diff`, `cargo vet inspect`).

## Step 2: Plan the Audit

Present the user with a table of all unvetted crates:

| Crate | Type | Audit Size | Notes |
|-------|------|-----------|-------|
| ... | delta/full | files/lines | ... |

Ask the user to confirm before proceeding.

## Step 3: Delegate to the Cargo-Vet Auditor Agent

For each crate, delegate the actual code review to the **cargo-vet-auditor** agent. Launch
multiple agents in parallel when there are many crates to audit.

Provide each agent with:
- The crate name and version(s)
- Whether this is a delta or full audit
- The exact command to run (`cargo vet diff CRATE FROM TO` or `cargo vet inspect CRATE VERSION`)
- The working directory

## Step 4: Compile Results

Collect agent results into a confidence score table:

| Crate | Type | Unsafe | Build/Proc Macro | Powerful Imports | Advertised Behavior | Confidence | Verdict |
|-------|------|--------|-----------------|-----------------|---------------------|------------|---------|

Confidence scoring rubric:
- **95-100**: No unsafe, no build script, no powerful imports, trivial/well-known crate
- **90-94**: Minimal unsafe (sound, reviewed), simple build script, well-understood crate
- **80-89**: Non-trivial unsafe (sound but complex), build script with FS access, larger crate
- **70-79**: Complex unsafe requiring careful review, proc macros with code generation
- **60-69**: Concerns noted but mitigated, unusual patterns
- **Below 60**: Red flags found — do NOT certify, escalate to user

## Step 5: Certify

### Exemptions Are a Last Resort

Do **not** add `[[exemptions]]` entries without explicit user confirmation.
Each exemption bypasses the audit process entirely and must be justified.
Valid reasons include:

- The crate is only needed for `safe-to-run` (test/dev tooling) and a full
  audit is disproportionate
- An upstream import source was removed and the crate needs temporary coverage
  while a first-party audit is scheduled
- The user explicitly requests an exemption after reviewing the trade-offs

Always prefer auditing (full or delta) over exempting. When an exemption is
unavoidable, present it to the user for manual approval before adding it.

Every exemption **must** include a `--notes` explaining why the exemption exists
and under what conditions it can be removed:

```shell
cargo vet add-exemption CRATE VERSION --criteria CRITERIA \
  --notes "Reason for exemption; plan for resolution"
```

For each crate that passes (confidence ≥ 70), run:

```shell
cargo vet certify CRATE FROM TO --accept-all --criteria safe-to-deploy \
  --who "NAME <EMAIL>" --notes "AUDIT_NOTES"
```

For full version audits (no delta), omit the FROM version:

```shell
cargo vet certify CRATE VERSION --accept-all --criteria safe-to-deploy \
  --who "NAME <EMAIL>" --notes "AUDIT_NOTES"
```

Use the git user's name and email for `--who`.

### AI Attribution in Audit Notes

Following the Linux kernel's AI attribution guidelines, every audit note MUST
include an `Assisted-by` tag to transparently disclose that the audit was
performed with AI assistance. Use the format:

```
Assisted-by: AGENT_NAME:MODEL_ID cargo-vet
```

Where:
- `AGENT_NAME` is `copilot-cli` (or the specific agent framework)
- `MODEL_ID` is the model that performed the review (e.g., `claude-sonnet-4.5`,
  `claude-opus-4.6`). Determine this from the session's model configuration.
- `cargo-vet` is the specialized analysis tool used

For example, a complete `--notes` value would be:

```
"No unsafe, no build script, no I/O. Assisted-by: copilot-cli:claude-opus-4.6 cargo-vet"
```

The human user remains responsible for reviewing all AI-generated audit
assessments and certifications. The `--who` field must always identify
the human reviewer, never the AI agent.

## Step 6: Verify and Clean Up

Before final verification, detect and remove identical duplicate `[[audits.*]]`
entries that may have been appended by retried `cargo vet certify` commands.

Duplicate-check workflow:

1. Scan `supply-chain/audits.toml` for byte-for-byte identical audit blocks
2. If duplicates exist, keep one copy (usually the first) and remove the rest
3. Re-run `cargo vet` after deduplication to ensure state is still valid

To detect duplicates, scan `supply-chain/audits.toml` for repeated blocks
with identical crate name, who, criteria, version/delta, and notes fields.
Remove any duplicates before proceeding.

Then run the normal cleanup sequence:

4. Run `cargo vet` again to confirm everything passes
5. Run `cargo vet prune` to remove stale exemptions
6. Run `cargo vet` one final time to confirm clean state

## Reviewing Import Sources

When asked to review imported audit sources (in `supply-chain/config.toml`), evaluate each on:

| Factor | Weight | How to Assess |
|--------|--------|---------------|
| Organization reputation | High | Known security-conscious org? (Mozilla, Google, Bytecode Alliance, etc.) |
| Activity / freshness | High | Last commit date, commit frequency |
| Community size | Medium | Stars, forks, contributors |
| Audit coverage | Medium | Number of unique crates audited |
| Domain relevance | Medium | Does their audit focus overlap with our dependency graph? |
| Dedicated audit repo | Low | Dedicated repo vs. audits inside a product repo |

Present results as a confidence score table with reasoning.
