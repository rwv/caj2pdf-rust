# Repository instructions

- The project's public language is English.
- Follow the GitHub issue hierarchy and blocked-by relationships. Read an
  issue's acceptance criteria before implementing it; report unmet criteria.
- All source code committed here must be MIT-licensed. Reimplement HN parsing
  and CAJ-specific JBIG decoding independently. Do not copy or transliterate
  code from the Python or Go converters, the private Rust prototype, or other
  differently licensed sources.
- Preserve memory-conscious I/O: seekable or ranged input, sequential output,
  bounded buffers, and temporary spooling for forward-only inputs when needed.
  Avoid whole-file `Vec<u8>` conversion APIs as the main path.
- Browser and Node.js are both first-class JavaScript targets. Keep platform
  adapters separate from conversion logic.
- Use Conventional Commits; mark breaking changes explicitly. `v0.x.y` is
  unstable and may contain documented breaking changes.
- Do not count skipped optional-corpus tests as successful compatibility tests.
  Do not commit the external CAJSamples document corpus to this repository.
