---
description: Rebase fork master onto rtk-ai/rtk upstream, surface conflicts and decision points, verify, then stop before push for review
---

# /sync-upstream

Reconcile this fork (`iliaal/rtk` on remote `fork`) with the canonical upstream
(`rtk-ai/rtk` on remote `origin`). Strategy is **rebase** — fork commits replay
on top of upstream so the history stays a linear overlay. The command stops
before pushing so the user reviews the force-push and any documentation
updates.

## When to invoke

- Periodic upstream sync (weekly / when upstream cuts a release).
- `git status` shows the local master is behind `origin/master`.
- After an upstream release-please tag bump appears in `git log origin/master`.

## Inputs

Optional argument: `--auto-push` to skip the manual approval gate at the end
(force-pushes to `fork` automatically). **Default is review-first.**

The command operates on the current branch's master. It refuses to run if HEAD
is not on `master` (the fork branch we sync).

## Implementation

Execute the script below. It is structured as discrete phases — fail fast at
any phase and stop, letting the human or the assistant decide the next step.

```bash
#!/usr/bin/env bash
set -euo pipefail

# ---- Configuration --------------------------------------------------------
UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-origin}"   # rtk-ai/rtk
FORK_REMOTE="${FORK_REMOTE:-fork}"             # iliaal/rtk
BRANCH="master"
TS="$(date +%Y%m%d-%H%M%S)"
BACKUP_BRANCH="master.bak.${TS}"

AUTO_PUSH=false
for arg in "$@"; do
  case "$arg" in
    --auto-push) AUTO_PUSH=true ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

bold() { printf "\033[1m%s\033[0m\n" "$*"; }
warn() { printf "\033[33m%s\033[0m\n" "$*"; }
fail() { printf "\033[31m%s\033[0m\n" "$*" >&2; exit 1; }

# ---- Phase 0: Pre-flight --------------------------------------------------
bold "[0/7] Pre-flight"

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || fail "not inside a git repo"

CUR_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$CUR_BRANCH" = "$BRANCH" ] || fail "not on $BRANCH (HEAD is $CUR_BRANCH)"

if [ -n "$(git status --porcelain)" ]; then
  fail "working tree not clean — commit or stash before syncing"
fi

git remote get-url "$UPSTREAM_REMOTE" >/dev/null 2>&1 || fail "remote '$UPSTREAM_REMOTE' missing"
git remote get-url "$FORK_REMOTE" >/dev/null 2>&1 || fail "remote '$FORK_REMOTE' missing"

UPSTREAM_URL="$(git remote get-url "$UPSTREAM_REMOTE")"
FORK_URL="$(git remote get-url "$FORK_REMOTE")"
echo "  upstream ($UPSTREAM_REMOTE): $UPSTREAM_URL"
echo "  fork     ($FORK_REMOTE):     $FORK_URL"

# ---- Phase 1: Fetch -------------------------------------------------------
bold "[1/7] Fetch"
git fetch --prune "$UPSTREAM_REMOTE"
git fetch --prune "$FORK_REMOTE"

BASE="$(git merge-base "$BRANCH" "$UPSTREAM_REMOTE/$BRANCH")"
AHEAD="$(git rev-list --count "$UPSTREAM_REMOTE/$BRANCH..$BRANCH")"
BEHIND="$(git rev-list --count "$BRANCH..$UPSTREAM_REMOTE/$BRANCH")"
echo "  common ancestor: $BASE"
echo "  fork ahead by $AHEAD, behind upstream by $BEHIND"

if [ "$BEHIND" = "0" ]; then
  echo "  ✓ already up to date with $UPSTREAM_REMOTE/$BRANCH — nothing to sync"
  exit 0
fi

# ---- Phase 2: Inventory ---------------------------------------------------
bold "[2/7] Inventory of incoming commits"
echo
echo "Fork-only commits (will be replayed on top):"
git log --oneline --no-decorate "$UPSTREAM_REMOTE/$BRANCH..$BRANCH" | sed 's/^/  /'
echo
echo "Incoming upstream commits:"
git log --oneline --no-decorate "$BRANCH..$UPSTREAM_REMOTE/$BRANCH" | sed 's/^/  /'
echo

# ---- Phase 3: Conflict prediction -----------------------------------------
bold "[3/7] Predicting conflicts"
UPSTREAM_FILES="$(git diff --name-only "$BASE..$UPSTREAM_REMOTE/$BRANCH" | sort -u)"
FORK_FILES="$(git diff --name-only "$BASE..$BRANCH" | sort -u)"
OVERLAP="$(comm -12 <(echo "$UPSTREAM_FILES") <(echo "$FORK_FILES"))"

if [ -z "$OVERLAP" ]; then
  echo "  ✓ no file overlap — rebase should apply cleanly"
else
  warn "  Files touched on both sides (potential conflict points):"
  echo "$OVERLAP" | sed 's/^/    /'
fi
echo

# ---- Phase 4: Backup ------------------------------------------------------
bold "[4/7] Backup current $BRANCH → $BACKUP_BRANCH"
git branch "$BACKUP_BRANCH" "$BRANCH"
echo "  (recover with:  git reset --hard $BACKUP_BRANCH  )"

# ---- Phase 5: Rebase ------------------------------------------------------
bold "[5/7] Rebase $BRANCH onto $UPSTREAM_REMOTE/$BRANCH"
if git rebase "$UPSTREAM_REMOTE/$BRANCH"; then
  echo "  ✓ rebase clean"
else
  warn "  Rebase paused due to conflicts."
  warn "  Resolve each file, then:"
  warn "    git add <file>... && git rebase --continue"
  warn "  Or abort:  git rebase --abort  (state will be restored)"
  warn "  Or roll back fully: git rebase --abort && git reset --hard $BACKUP_BRANCH"
  exit 1
fi

# ---- Phase 6: Verify ------------------------------------------------------
bold "[6/7] Verify (cargo fmt --check, clippy --deny warnings, test)"
# Upstream CI now denies clippy warnings (commit 70b9f38), match that locally.
set +e
cargo fmt --all -- --check
FMT_RC=$?
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -40
CLIPPY_RC=${PIPESTATUS[0]}
cargo test --all 2>&1 | tail -40
TEST_RC=${PIPESTATUS[0]}
set -e

VERIFY_FAILED=false
[ "$FMT_RC"    != "0" ] && { warn "  fmt FAILED";    VERIFY_FAILED=true; }
[ "$CLIPPY_RC" != "0" ] && { warn "  clippy FAILED"; VERIFY_FAILED=true; }
[ "$TEST_RC"   != "0" ] && { warn "  test FAILED";   VERIFY_FAILED=true; }

if [ "$VERIFY_FAILED" = "true" ]; then
  warn "  Verification failed. Backup branch preserved: $BACKUP_BRANCH"
  warn "  Investigate failures before pushing."
  exit 1
fi
echo "  ✓ fmt + clippy + test all green"

# ---- Phase 7: Summary + push gate -----------------------------------------
bold "[7/7] Summary"
NEW_HEAD="$(git rev-parse --short HEAD)"
OLD_HEAD="$(git rev-parse --short "$BACKUP_BRANCH")"
echo "  master:  $OLD_HEAD → $NEW_HEAD"
echo "  picked up $BEHIND upstream commit(s), replayed $AHEAD fork commit(s)"
echo "  backup branch: $BACKUP_BRANCH (delete with: git branch -D $BACKUP_BRANCH)"
echo
echo "Doc-impact scan (review these manually):"
echo "  - New top-level subcommands in $UPSTREAM_REMOTE/$BRANCH:"
git log "$BASE..$UPSTREAM_REMOTE/$BRANCH" --format='%s' \
  | grep -iE '^(feat|feature)(\([^)]+\))?:' \
  | sed 's/^/      /' || true
echo
echo "  - Breaking/security/CI commits worth noting:"
git log "$BASE..$UPSTREAM_REMOTE/$BRANCH" --format='%h %s' \
  | grep -iE '(breaking|security|cicd|deny warnings|migration|deprecat)' \
  | sed 's/^/      /' || true
echo
echo "  - Files in this fork that may need a doc refresh:"
echo "      README.md, CLAUDE.md, INSTALL.md          (this repo)"
echo "      ~/.claude/RTK.md                          (Claude Code rtk rules — hand-edited, persists)"
echo "      ~/.codex/AGENTS.md                        (Codex CLI agent rules — hand-edited, persists)"
echo "      ~/.codex/RTK.md                           (Codex auto-generated; do NOT hand-edit — regenerated by 'rtk init --agent codex')"
echo "      hooks/codex/rtk-awareness.md  (this repo) (source template for ~/.codex/RTK.md — edit here to ship richer Codex defaults)"
echo "      ~/ai/wiki/tools/rtk.md                    (cross-repo wiki)"
echo

if [ "$AUTO_PUSH" = "true" ]; then
  bold "Pushing to $FORK_REMOTE (--auto-push)"
  git push --force-with-lease "$FORK_REMOTE" "$BRANCH"
  echo "  ✓ pushed"
else
  bold "Push gate — review the above, then run:"
  echo "    git push --force-with-lease $FORK_REMOTE $BRANCH"
  echo
  echo "  (force-with-lease refuses the push if someone else updated the fork remote)"
fi
```

## Assistant-side responsibilities (when invoked via Claude Code)

The bash script does the mechanical work. The assistant should also:

1. **Read upstream commit messages and diffs** for the incoming range printed
   in Phase 2 to identify substantive features, behavior changes, or new
   subcommands worth surfacing.

2. **For each conflict** during rebase, fetch both sides of the conflict
   (`git show :2:<path>` and `:3:<path>`), reason about intent, propose a
   merge that preserves both fork additions and upstream changes, and stop
   for confirmation before staging it. Do not blindly accept either side.

3. **Doc update analysis.** After a successful rebase, scan the upstream
   commit set for changes that may make existing docs stale. Treat agent
   instructions as a *set* — Claude Code and Codex CLI both consume rtk,
   so behavior notes worth telling one agent are usually worth telling the
   other. Check each location below in parallel:

   - **Fork repo docs** (`README.md`, `CLAUDE.md`, `INSTALL.md`,
     `src/cmds/<eco>/README.md`) — new subcommands, supported-ecosystem
     list bumps, version pins, install path changes.
   - **`~/.claude/RTK.md`** — hand-edited Claude Code agent rules.
     Persistent; edit directly. Behavior notes the model needs at
     generation time (e.g. wrapper-command transparency, bypass
     discipline, new subcommand semantics).
   - **`~/.codex/AGENTS.md`** (RTK section near line ~193) — hand-edited
     Codex CLI agent rules. **This is the right place for any note that
     parallels a `~/.claude/RTK.md` change.** Persistent; edit directly.
   - **`~/.codex/RTK.md`** — *auto-generated* from
     `hooks/codex/rtk-awareness.md`. Do not hand-edit; `rtk init
     --agent codex` overwrites it. If a Codex default deserves to ship
     for everyone, edit the template in this repo and let the next
     install pick it up. If it's local-only guidance, put it in
     `~/.codex/AGENTS.md` instead.
   - **`~/ai/wiki/tools/rtk.md`** — cross-repo wiki entry. Always append
     a sync log line to `~/ai/wiki/log.md` and refresh the frontmatter
     `description` if the page's surface area changed (BM25 retrieval
     depends on it). Update the install-source / version row in the
     `## Measurements` table.

   Triggers (non-exhaustive):
   - Hook / init behavior change (`transparent_prefixes`, `--dry-run`,
     `permissionDecision` semantics, new agent target) → update
     `~/.claude/RTK.md` AND `~/.codex/AGENTS.md` in parallel, plus the
     wiki.
   - Function signature changes that ripple to fork tests (e.g.
     `rewrite_command` arity bumps) → note in the wiki sync log so the
     pattern is captured for next time.
   - Security pins / workflow restructuring → likely repo-internal only.
   - Breaking changes → fork `CLAUDE.md`, fork `INSTALL.md`, both agent
     files, and the wiki.

   Propose specific edits; do not apply them until the user approves.

4. **Branch hygiene.** Once the user confirms the force-push succeeded and is
   happy with the result, delete `master.bak.<ts>` with the user's go-ahead.
   Do not delete the backup branch automatically.

## Safety properties

- Refuses to run with a dirty working tree.
- Creates a timestamped backup branch before touching history; rollback is
  one `git reset --hard <backup>` away.
- Uses `--force-with-lease` (never `--force`) on push — refuses if the fork
  remote moved underneath.
- Verification gate (`cargo fmt --check` + `clippy -D warnings` + `cargo
  test --all`) is mandatory and matches upstream CI's gate (commit `70b9f38`
  on `origin/master` made clippy warnings a hard fail).
- Does not push unless `--auto-push` was passed explicitly.

## Non-goals

- This command does not cherry-pick PR branches (`origin/feat/*`,
  `origin/fix/*`) — only `origin/master`. PR branches are integrated upstream
  by their authors; pulling pre-merge PRs into the fork creates a divergent
  history.
- This command does not run `cargo install --path .` to refresh the local
  binary. After a successful sync, the user should run `cargo install --path
  .` (or `cargo build --release && cp target/release/rtk ~/.local/bin/`)
  separately.
- This command does not regenerate or edit `CHANGELOG.md` for the fork — the
  fork tracks upstream releases via rebase rather than maintaining its own
  changelog.

## Recovery cheatsheet

- **Rebase went sideways**: `git rebase --abort` then
  `git reset --hard master.bak.<ts>`.
- **Pushed bad rebase to fork**: re-checkout the backup,
  `git push --force-with-lease fork master.bak.<ts>:master`.
- **Lost the backup branch**: `git reflog` shows the previous master HEAD;
  recover with `git reset --hard <reflog-sha>`.
