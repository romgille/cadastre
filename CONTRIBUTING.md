# Contributing

Thanks for looking. This project is maintained on a best-effort basis; pull
requests and issues are welcome and will be read, but not always quickly.

## Before you start

For anything beyond a small fix, open an issue first so that the direction can
be agreed before you spend time on it. Issues labelled `good first issue` and
`help wanted` are a good place to start.

## What a good change looks like

- **Small, atomic commits.** Each commit builds, passes the tests and does one
  thing. A reviewer should be able to read the history like a story.
- **Conventional commit subjects**: `feat(scope): …`, `fix(scope): …`,
  `test(scope): …`, `docs: …`, `ci: …`, `build: …`. The body says why, not
  what.
- **Tests with the code.** A bug fix starts with a test that fails.
- **No secrets in fixtures that look real.** Fixtures use the `LEAK-` prefix so
  the canary test can detect leaks; keep that convention.

## The rule that matters most

Anything a container reports can contain a secret. New output must be built
from fields of the sanitized model (`src/model.rs`), never from the raw inspect
document. If you add a field, add a canary for it in `tests/fixtures` and
confirm `tests/secrets.rs` still passes for the right reason.

Equally important: **say how a change fails.** If it can fail, does the
operator find out? Prefer designs where the answer is "loudly". See
[docs/failure-modes.md](docs/failure-modes.md).

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

CI runs the same, plus `cargo deny check` and a build of the container image.
`unwrap` and `expect` are denied outside tests: a panic in a background thread
is a silent failure.

No Docker daemon is needed for the tests; they use a small mock of the Engine
API (`tests/support/mock_docker.rs`).

## Licence

By contributing you agree that your work is licensed under
[AGPL-3.0-or-later](LICENSE).
