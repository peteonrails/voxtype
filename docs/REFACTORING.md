# Refactoring policy

This is for me when I'm planning releases. Contributor-side cleanup is
covered in [`CONTRIBUTING.md`](../CONTRIBUTING.md); the difference is
that contributors clean as they go, and I sometimes ship a whole release
whose point is structural work.

I treat a tidy codebase as a feature. The cost of letting structure
decay shows up later as slower feature PRs, recurring bug classes, and
maintainer reluctance to re-enter modules I haven't touched in months.
So refactoring competes with features by being scheduled like them, not
by being squeezed in around them.

## When a release is refactor-themed

Most releases ship features. Some ship structural work instead. I pick
a refactor theme when one of these is true:

A bug class has recurred. The fix patched a symptom and the mechanism
that produced it is still there. The liveness check duplication is the
canonical example: fixed once on the CLI side, still latent in the TUI.

The next feature on the roadmap would force enough structural change
that doing the structural work first is cheaper than smuggling it into a
feature PR.

A specific file or function has grown to the point where I burn ten or
fifteen minutes rebuilding context every time I open it. `daemon.rs::run()`
at 1580 lines is in this category.

I won't ship a refactor theme just because a release is short on
features. There has to be a real problem to remove.

## What gets into the release

I admit refactor items by one of five tests. An item earns its spot if
at least one holds:

1. It's a correctness risk. Duplicate sources of truth, untested critical
   paths, a class of bug that has surfaced once and could come back.
2. It's blocking something on the upcoming roadmap. Feature pressure
   pulls the structural work in.
3. The same change keeps coming up in PRs. Every new flag adds another
   override-block copy; every new engine adds another download function.
4. A file's name no longer predicts its contents. The split is along
   meaning (subcommands, event types, data vs logic), not line counts.
5. The same fact is written in two places that can drift. Code that
   merely looks similar but answers different questions is not the same
   fact; leave it alone.

The sixteen sentinel files under `runtime_dir/` (`model_override`,
`cancel`, `meeting_state`...) look alike but are unrelated; merging them
would invent write-race surface that doesn't exist today. The
daemon-liveness check, by contrast, asks one question in three places,
and the three places disagree on macOS.

If none of the five tests holds, the refactor doesn't make the release.

## How items have to be specified

Every item in scope needs a done test that's a yes or no. "Consolidated
the liveness check" is a vibe; "no file outside `daemon.rs` references
the legacy `pid` filename, asserted by a test" is a done test.

Before changing code that doesn't already have tests, write one that
pins the current behaviour. The refactor has to keep every test that
passed before. If a refactor changes behaviour, it isn't a refactor; it
just looks like one.

Lock the result in with something the compiler or CI enforces. A
deletion that makes the wrong path impossible (the strum-derived
`TranscriptionEngine::name()`) is better than a comment saying "use the
new helper". A guard test that fails when an old pattern reappears is
better than relying on discipline.

If a refactor item grows past its planned size, stop and split it.

## Things to avoid

I won't ship a refactor whose justification is that the file is large
or that the code is ugly. Large and stable is fine. Ugly and unused is
fine.

I won't bundle behaviour changes inside a refactor PR. If a "while I'm
here" tweak slips in, the PR isn't behaviour-preserving and review
costs go up sharply. They go in separate PRs.

I won't extract abstractions from a single call site. The first version
of an abstraction is almost always wrong about which axis varies; wait
for two or three concrete uses before pulling out a trait or a generic.

## What to track

Three rough numbers to glance at periodically: how many places encode
the same fact, how many files I have to touch to add a new model or
engine, and how much of the config surface is exposed by default versus
buried under `[advanced]`. Lower is better on all three.
