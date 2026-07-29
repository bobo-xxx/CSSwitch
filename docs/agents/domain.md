# Domain Docs

Use this repository's domain documentation before exploring or changing code.

## Read first

- Read `CONTEXT.md` at the repository root when it exists.
- Read relevant ADRs under `docs/adr/` when they exist.
- If either location is absent, proceed without creating placeholder content.

## Layout

This is a single-context repository:

```text
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Vocabulary and decisions

- Use the terms defined in `CONTEXT.md`, including their stated boundaries and avoided synonyms.
- If a proposed change conflicts with an ADR, surface the conflict explicitly instead of silently overriding it.
