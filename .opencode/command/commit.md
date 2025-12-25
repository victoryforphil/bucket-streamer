---
description: Summarize changes and create organized commits
---

# Check Current State

Run these commands in parallel to see the current git state:

```bash
git status
git diff --cached
git diff
git log -5 --oneline
```

# Summarize Changes

Analyze the changes and provide a brief summary to the user of what was changed.

# Organize Commits

Group changes logically into commits following the format:

```
Component // [SubSystem] // Title [info]
```

## Commit Organization Strategy

- **Config // Project** - Configuration files (opencode.json, .opencode rules)
- **Docs // Agents** - AGENTS.md and agent-related documentation
- **Docs // Design** - Design docs (design_stage1.md, architecture notes)
- **Docs // Tasks** - Task definitions and implementation guides
- **Server // Component** - Server components (router, websocket, etc.)
- **CLI // Component** - CLI components and tools
- **Pipeline // Component** - Pipeline components (decoder, encoder, etc.)
- **Storage // Component** - Storage and backend implementations

# Create Commits

For each logical group:

1. Stage the files: `git add <files>`
2. Create commit: `git commit -m "Component // [SubSystem] // Title [info]"`
3. Verify with: `git status` and `git log --oneline -N`

# Verification

After all commits, verify:

```bash
git status
git log --oneline -<commit_count>
```

# Push (With User Approval)

Before pushing, always ask the user:

```
Would you like to push these changes to the remote repository?
```

If user approves, run:

```bash
git push
```

# Key Rules

1. **Format Consistency**: Always use `Component // [SubSystem] // Title [info]` format
2. **Logical Grouping**: Group related files together in commits
3. **Atomic Commits**: Each commit should represent one logical change
4. **Ask Before Push**: Never push without explicit user approval
5. **Clean State**: Ensure working tree is clean before finishing
