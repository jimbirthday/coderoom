# Repository Guidelines

## Project Structure & Module Organization
This repo is currently scaffold-light (no standard directories detected). As you add code, prefer a predictable layout:
- `src/`: application/library source code
- `tests/`: unit/integration tests mirroring `src/`
- `scripts/`: developer automation (release, migrations, fixtures)
- `docs/`: design notes and ADRs
- `assets/` or `public/`: static files (if applicable)

Keep new modules small, single-purpose, and grouped by domain (e.g., `src/auth/`, `src/billing/`) rather than by “utils”.

## Build, Test, and Development Commands
Add a `Makefile` (or equivalent) that wraps the project’s primary toolchain so contributors have one entry point:
- `make setup`: install dependencies and prepare local config
- `make fmt`: format code
- `make lint`: run static checks
- `make test`: run the full test suite
- `make run`: start the app locally
- `make build`: produce release artifacts

If you don’t use `make`, mirror these as `npm run …`, `just …`, or similar and document them here.

## Coding Style & Naming Conventions
- Indentation: follow the repo formatter; avoid manual formatting.
- Naming: `kebab-case` for filenames, `PascalCase` for types/classes, `camelCase` for variables/functions.
- Prefer explicit names over abbreviations; avoid “misc”/“helpers” grab-bags.

## Testing Guidelines
- Place tests in `tests/` and mirror module paths (e.g., `src/foo/bar` → `tests/foo/bar`).
- Name tests consistently with your framework (e.g., `test_*.py`, `*.test.ts`).
- Keep unit tests fast; gate slower integration tests behind a separate command/flag.

## Commit & Pull Request Guidelines
No Git history is available yet; use Conventional Commits:
- `feat: …`, `fix: …`, `docs: …`, `refactor: …`, `test: …`, `chore: …`

PRs should include: summary + motivation, how to test, linked issue(s), and screenshots for UI changes. Call out breaking changes and config/env var updates.

## Security & Configuration Tips
- Never commit secrets. Use `.env.example` for defaults; keep `.env` local and gitignored.
- Document required environment variables and local setup steps in `README.md`.
