# Issue tracker: Local Markdown

Issues and specs for this repository live as Markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The spec is `.scratch/<feature-slug>/spec.md`
- Implementation issues are one file per ticket at `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`
- Triage state, when needed, is recorded as a `Status:` line near the top of each issue file
- Comments and conversation history append under a `## Comments` heading

## Publishing and fetching

- To publish to the issue tracker, create a file under `.scratch/<feature-slug>/`, creating the directory when needed.
- To fetch a ticket, read the referenced path or numbered issue file.

## Wayfinding operations

- **Map:** `.scratch/<effort>/map.md` contains Notes, Decisions so far, and Fog.
- **Child ticket:** `.scratch/<effort>/issues/NN-<slug>.md` contains one question. `Type:` is `research`, `prototype`, `grilling`, or `task`; `Status:` is `open`, `claimed`, or `resolved`.
- **Blocking:** `Blocked by: NN, NN` names prerequisite tickets. A ticket becomes unblocked when each listed ticket is resolved.
- **Frontier:** choose the lowest-numbered open, unblocked, unclaimed ticket.
- **Claim:** change `Status:` to `claimed` before starting work.
- **Resolve:** append the answer under `## Answer`, set `Status:` to `resolved`, and add a gist plus link to the map's Decisions so far.
