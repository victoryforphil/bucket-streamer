# Git Commit & Push Workflow

## Commit Format
```
Component // [SubSystem] // Title [info]
```

Examples:
- `Server // Created base server`
- `CLI // Convert // Fixed h265 convertor (hotfix)`

## Branch Strategy
- `main` for normal development
- `vfp/agent/<name>` for risky R&D only (can revert)

## Workflow
1. **Session start**: Ask if user wants commits at checkpoints
2. **Plan mode**: Don't commit (unless user asks)
3. **Build mode**: Commit at logical checkpoints with proper format
4. **Before push**: Always ask user first (never push silently)

## Key Rules
- Use format consistently
- Never push without approval
- Commits are checkpoints for validation
