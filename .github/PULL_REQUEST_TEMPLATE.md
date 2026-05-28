<!--
PRs target the `dev` branch, not `main`.

Branch flow: `dev` (default for PRs) -> `rc/x.y.z` (release candidate, triggers
the full build matrix) -> `main` (release-tagged) -> tag `vX.Y.Z`.

Open your PR as a draft while CI is red. Mark Ready for Review once the
`ci-success` aggregator check passes.
-->

## Description

Brief description of the changes in this PR.

## Related Issue

Fixes #(issue number)

## Type of Change

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to change)
- [ ] Documentation update

## Testing

- [ ] I have tested these changes locally
- [ ] I have run `cargo test` and all tests pass
- [ ] I have run `cargo clippy -- -D warnings` with no warnings
- [ ] I have run `cargo fmt`

## Documentation

- [ ] I have updated documentation as needed
- [ ] No documentation changes are needed

## Pre-merge checklist

- [ ] This PR targets the `dev` branch (not `main`)
- [ ] Wait for `ci-success` to pass before marking Ready for Review (draft mode is encouraged while CI is red)

## Additional Notes

Any additional information reviewers should know.
