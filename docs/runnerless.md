# Repository automation

The public repository is [no-std-rs/poznan](https://github.com/no-std-rs/poznan).
The local layout uses `poznan/.git` as the bare repository, `poznan/main` for
`main`, and named topic worktrees such as `poznan/waste-stream`.

GitHub Actions runs Rust formatting, Clippy, tests, documentation and packaging.
The MSRV job builds the library on Rust 1.91.1 for the host, bare-metal RISC-V
and WebAssembly. Pull request titles follow Conventional Commits. Dependencies
and pinned GitHub Actions receive weekly Dependabot updates.

Runnerless evaluates Codex review evidence using `.runnerless-ci.ts`. The
`codex-review` workflow reacts to pull request, review, comment, thread and
check events, and reports `Runnerless / Codex review`. The program comes from
the pull request's trusted base revision. Missing or unsuccessful review evidence
does not pass. After pushing fixes, the current head needs a fresh review.

## Enrollment and verification

The repository must be connected to the organization's existing My Toolkit
GitHub App installation (App ID `4602759`) and adopted in Runnerless. With an
authorized `runl` session, enrollment is:

```sh
runl repositories add no-std-rs/poznan --github-id 1424884458
```

The CLI accepts the GitHub verification credential through hidden input or
stdin; keep it out of command arguments, source files and logs. Enrollment,
repository automation preferences and GitHub App access are external settings;
committing a program alone does not activate them. Current Runnerless enrollment
does not require adding a repository to infrastructure allowlists.

Runnerless's Repository automation panel controls automatic review requests,
managed protection and administrator bypass. GitHub's required checks must keep
the Rust jobs and `Conventional Commits`. Require an App-owned Runnerless check
only after verifying that the service actually reports it. Preserve existing
checks when updating protection.

Validate activation with `runl repositories list`, the repository's automation
settings and a live pull request check. The application's review tracker can
also report `codex/review-gate`; inspect the actual check name and App before
making it required. An enrolled repository and a successful build alone do not
prove that Codex review enforcement is active.
