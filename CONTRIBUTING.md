# Contributing

Thanks for wanting to help. Issues with a clear "what I did, what I expected, what happened" are the most useful
thing you can send; say which AI account and which messaging app the bot was on.

**Pull requests.** Small and focused, with a test for the behaviour you changed (`cargo test --workspace`, and
`npx tsc -b` in `ui/`). Update the matching page in `docs/` when what people see changes, and add a line to
`CHANGELOG.md` in the words someone using Mimir would use.

**Licence.** Mimir is AGPL-3.0-only. Before the first outside pull request is merged we'll ask contributors to sign a
contributor licence agreement (through CLA Assistant, one click), so the project can keep offering other licences
later. Until then, please open an issue first if you're planning something large.

**Security.** Please don't open public issues for security problems: use GitHub's private vulnerability reporting on
this repository instead. [docs/security.md](docs/security.md) describes what Mimir defends against.
