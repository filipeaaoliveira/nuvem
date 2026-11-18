## What

<!-- One or two sentences: what changes and why. -->

## Checklist

If you cannot run the gates locally, tick what you ran and say so — CI runs
the same gates on your PR.

- [ ] `scripts/check-fast.sh` passes (fmt, clippy `-D warnings`, shellcheck, suite)
- [ ] Domain logic changes have their tests in `crates/nuvem-domain`, beside the code they pin
- [ ] New integration test files are mounted in the relevant `tests/suite.rs`
- [ ] `ARCHITECTURE.md` updated if modules moved or were added (its tree is machine-checked)
- [ ] User-visible behaviour changes are reflected in `README.md` and `CHANGELOG.md`
