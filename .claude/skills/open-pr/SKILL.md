---
name: open-pr
description: Use when creating a git branch, pushing work, or opening a pull request in this repo. Enforces human-readable branch names and filling in the PR template.
---

# Opening a pull request

## Branch names

Name branches so a reviewer knows what they're about without opening them.

- Format: `<type>/<short-kebab-description>`, where type is one of `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.
- Describe the change, in 3 to 6 words: `fix/menu-page-wrong-face`, `feat/result-orientation-lock`, `docs/pr-template`.
- No random suffixes, session IDs, or auto-generated names (`peaceful-clarke-4rxpdi`, `patch-1`, `wip`).
- Lowercase, hyphens only, no spaces or underscores.
- Branch from `main`.

If the environment or the user designates a specific branch (for example a
session that says "develop on branch X"), use that branch as given. Don't
create a differently named one, and don't rename it without asking. Suggest a
readable name for next time instead.

## Pull request description

Every PR must use `.github/pull_request_template.md`.

1. Read the template from the repo before writing the body. Don't work from memory; it may have changed.
2. Keep its section headings and order: Summary, Packages touched, How it was tested, Checklist.
3. Fill in every section from the actual diff:
   - Summary: what and why, linking the issue or spec section (e.g. SIM_SPEC H9).
   - Packages touched: tick only the ones the diff changes.
   - How it was tested: the real commands you ran and their results. If you didn't run something, say so; don't tick it.
   - Checklist: tick only items you verified. Leave the rest unticked and explain why.
4. Don't delete sections or replace the body with a free-form description. If a section doesn't apply, write "N/A" with a reason.
5. The title is a short imperative sentence describing the change, like a commit subject.

## Before opening

- `git diff main...HEAD --stat` to confirm the diff matches what the description says.
- Run the checks listed in the README's Contributing section for the packages touched.
- Don't open a PR unless the user asked for one.
