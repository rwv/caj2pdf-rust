# Repository instructions

- The project's public language is English.
- Follow the GitHub issue hierarchy and blocked-by relationships. Read an
  issue's acceptance criteria before implementing it; report unmet criteria.
  The v0.1.0 entry point is the [parent issue](https://github.com/rwv/caj2pdf-rust/issues/1).
- All source code committed here must be MIT-licensed. Reimplement HN parsing
  and CAJ-specific JBIG decoding independently. Do not copy or transliterate
  code from the Python or Go converters or other differently licensed sources.
  Migrate a private Rust module only after per-file provenance review confirms
  original ownership and MIT eligibility; never migrate its JBIG/HN code.
- Keep I/O bounded, not spooled: input is ranged or seekable and is never
  read whole; output is sequential. One image payload, one symbol dictionary
  or one page bitmap may be held in memory under `Limits`. Forward-only input
  is spooled by the platform adapter, not the core.
- Browser and Node.js are both first-class JavaScript targets. Keep platform
  adapters separate from conversion logic.
- Use Conventional Commits; mark breaking changes with `!` or a
  `BREAKING CHANGE:` footer. `v0.x.y` is unstable and may contain documented
  breaking changes.
- Do not count skipped optional-corpus tests as successful compatibility tests.
  Do not commit the external CAJSamples document corpus to this repository.
- Keep [docs/provenance.md](docs/provenance.md) current for format facts,
  fixtures, dependencies, and every proposed private-source migration. Follow
  [docs/release-policy.md](docs/release-policy.md) for review and releases.
