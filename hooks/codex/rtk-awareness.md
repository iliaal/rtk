# RTK - Rust Token Killer (Codex CLI)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

Always prefix shell commands with `rtk`.

Examples:

```bash
rtk git status
rtk cargo test
rtk npm run build
rtk pytest -q
```

## Meta Commands

```bash
rtk gain            # Token savings analytics
rtk gain --history  # Recent command savings history
rtk proxy <cmd>     # Run raw command without filtering
```

## When to use `rtk proxy`

Use filtered output for routine navigation. When filtering omits or alters
specific evidence the current check depends on, use a recoverable raw artifact
or `rtk proxy`, including when you are the one inspecting that evidence:
patches, hashes, byte comparisons, exact counts, file extraction
(`git show <sha>:<path>`, `git archive`), and reviewer or validator bundles.

Separately and unconditionally: when command output becomes input to another
command, parser, or reviewer, use raw output from the outset rather than
discovering the problem after a parse fails. That covers parsers, patches,
hashes, byte comparisons, exact counts, file extraction, and reviewer or
validator bundles. Never use filtered output to rewrite a file or feed a strict
parser.

Use `rtk git diff` for an overview and `rtk proxy git diff` when preparing an
exact patch or reviewer input. RTK rewrites only a pipeline's final stage, so
`git diff | head -50` leaves `git diff` itself unfiltered unless you prefix
that stage yourself.

## Verification

```bash
rtk --version
rtk gain
which rtk
```
