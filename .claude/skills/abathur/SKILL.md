---
name: abathur
description: Generate and maintain indexed abathur documentation for the project
---

# Abathur Documentation Skill

Use this skill to generate, update, or check abathur documentation.

## Prerequisites

The `abathur` CLI must be available. Build it with:
```bash
buck2 build root//src/abathur-cli:abathur-cli
buck2 run root//src/abathur-cli:install
```

## Workflow: Check for stale docs

```bash
abathur check
```

If stale docs are found, follow the "Update existing doc" workflow below.

## Workflow: Create a new doc

1. Get the schema:
```bash
abathur code
```

2. Read the source files you want to document.

3. Write a new `.md` file in `docs/abathur/` following the schema from step 1.
   - Set `hash: ""` for all sources - the CLI will compute them.
   - List all `## Heading` names in the `sections` field.

4. Update hashes:
```bash
abathur hash docs/abathur/<new-doc>.md
```

5. Verify:
```bash
abathur check
```

## Workflow: Update existing doc

1. Identify what changed:
```bash
abathur check --format json
```

2. Read the changed source files to understand the updates.

3. Read the current doc:
```bash
abathur read <slug>
```

4. Edit the doc file to reflect the source changes.

5. Update hashes:
```bash
abathur hash docs/abathur/<doc>.md
```

6. Verify:
```bash
abathur check
```

## Querying docs

```bash
abathur query "search terms"        # text search
abathur query "api" --tag            # tag search
abathur read <slug>                  # full doc
abathur read <slug> --section <name> # specific section
```

Add `--format json` to any command for structured output.
