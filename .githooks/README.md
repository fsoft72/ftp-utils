# Git hooks

Version-controlled hooks for this repo. Activated per clone by pointing git at
this directory:

```sh
git config core.hooksPath .githooks
```

## Hooks

- **pre-commit** - refreshes the codegraph index (`codegraph index --quiet`)
  when `codegraph` is on `PATH`. It is an optional convenience: if codegraph is
  missing or the index run fails, the hook logs a note and the commit proceeds.
- **prepare-commit-msg** - `core.hooksPath` makes git ignore any globally
  configured hooks, so this re-invokes a global `prepare-commit-msg` (from
  `git config --global core.hooksPath`, else `~/.git-templates/hooks`) if one
  exists.

## Bypass

`git commit --no-verify` skips all hooks.
