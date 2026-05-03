# Overview
Ludared is a workspace-driven binary transformation system designed for romhacking and archive manipulation.

It operates on:
- binary artifacts (ROMs, ISOs, archives)
- derived views (JSON, PNG, extracted files)
- user-edited sources

Goal: reproducible, portable, and structured pipelines.

***
# Core Principles
- Pipeline-first, not GUI-first
- Reproducibility over convenience
- Explicit over implicit
- Cross-platform by design
- CLI is the source of truth, GUI is a layer

***
# Artifact Model
Ludared operates on transformations of binary data:
```
code.bin
→ slice
→ compressed blob
→ parsed structure
→ editable JSON
```

Reverse path is also defined.

***
# Workspace Layout
```shell
.
├── sources
│   ├── user-provided-input (not versioned)
├── cache
│   ├── extracted/intermediate files (not versioned)
├── tools
│   ├── bundled tools / scripts / binaries (versioned)
├── src
│   ├── files being worked on (versioned)
└── project.ludared (versioned)
```

***
# Source Resolution
Artifact resolution priority:
1. src/
2. cache/

Meaning:
- src = authoritative user edits
- cache = fallback extracted data

***
# Workflow
Setup: 
```bash
$ git clone
# copy inputs to sources
$ ludared extract
$ ludared build
```

***  
# Handlers
Handlers define transformations:
- builtin
- command (external tools)
- (future) wasm
Multiple handlers can apply to a single artifact.

***
# Tools System
Tools are declared in project:
- Bundled (preferred)
- path-based (explicit requirement)

No implicit PATH usage.

***
# Requirements
Projects can declare requirements:
- executable presence
- version constraints

Validated via:
```bash
$ ludared doctor
```

***
# Build System
Ludared replaces Make:
- Dependency graph
- Incremental rebuild
- Domain-specific logic

Makefile optional (debug/export only).
***
# Versioning Strategy
Single project supports multiple targets:
```json
{
  "targets": [
    "v1_1",
    "v1_2`"
  ]
}
```

Shared pipeline, optional overrides.

***  
# Multi-Node Pipelines
A single logical transformation can expand into multiple steps:
```
code.bin
→ levels.lz
→ level_%d.bin
→ level.json
```
Internally multi-step, externally may appear as one node.

***  
# CLI + GUI Architecture
* `ludared-core` → Logic
* `ludared-cli` → Execution
* `ludared-app` → GUI

GUI only configures and triggers.

***
# CI / Automation
Can be used with GitHub Actions for weekly builds.

Flow:
- fetch original
- run ludared
- publish output

# Plugins (Future)
Not needed in v1/v2.

Future options:
- external tools (already supported)
- WASM plugins (recommended later)

Avoid native dynamic Rust plugins.

***
# Design Philosophy
- Do not over-abstract early
- Keep pipelines explicit
- Prefer portability over convenience
- Treat everything as data transformations

***
# Summary
Ludared needs to be:
- a binary-aware build system
- a reproducible workspace
- a pipeline engine

And not:
- just a GUI tool
- a thin wrapper over scripts
- platform-dependent