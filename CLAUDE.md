# glyphwave

## Branches

- Every agent or task starts its own branch off an up-to-date `main`, named
  for the feature (a worktree branch is fine).
- Work and commit on that branch. When the work is done and verified, merge
  it into `main` and push `origin main`.
- No long-lived shared branch collects features. Don't commit to whatever
  branch happens to be checked out; check `git branch` first.
