# Syncing upstream into Shuvbolt

Review upstream on a regular cadence (weekly when active, and promptly for
security fixes). Keep `dev` an exact mirror of `upstream/dev`; never develop on
it. The fork base was `anomalyco/deadbolt` at `9e76cf2`; preserve `74b0257` in
fork ancestry even if upstream later takes an equivalent fix.

Start clean with ordinary Git. `upstream` points to `anomalyco/deadbolt`,
`origin` to `shuv1337/deadbolt`:

```sh
git fetch upstream dev
git fetch origin shuv/dev
git switch dev
git merge --ff-only upstream/dev
test "$(git rev-parse dev)" = "$(git rev-parse upstream/dev)"
git switch -c "sync/upstream-$(TZ=America/Los_Angeles date +%Y%m%d)" origin/shuv/dev
git merge --no-ff dev
git merge-base --is-ancestor 74b0257 HEAD
```

Stop on a failed fast-forward or mirror check. Resolve conflicts against
[FORK_CONTRACT.md](FORK_CONTRACT.md): preserve protocol, vault, and native
messaging identifiers; keep the fork display/trust identities and no-release
policy; review conflicts in the Retry fix rather than blindly choosing a side.
Check changed identifiers with `git grep`, and re-review any changes to
upstream release/install workflows or updater entry points.

```sh
bun run check:fork
bun run check:proto
bun run check
cargo +stable clippy -p deadbolt-cli --features upstream-update-fixtures --all-targets --locked -- -D warnings
cargo +stable test -p deadbolt-cli --features upstream-update-fixtures --test cli --locked -- updates
cargo +stable check -p deadbolt-net --features private-relay-fixture --locked
cargo +stable test -p deadbolt-net --test net a_handshake_raced_over_two_paths_connects --locked
gh workflow list --all --repo shuv1337/deadbolt
```

Verify **Deploy site (dev)** (`431984918`) and **Release** (`431984919`) are
disabled in the fork as well as guarded in YAML. Run the fork smoke tests in
disposable XDG and `DEADBOLT_HOME` directories; never use a live vault or daemon.
Push only the sync branch for a PR to `shuv/dev` on `shuv1337/deadbolt`. Merge
with a merge commit after review, not by squashing or rebasing. Never push to
`upstream`.
