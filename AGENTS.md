# LLM (AI) Code in This Project

ludared is a passion project. Having an LLM design and implement the project
on its own would defeat much of the purpose and enjoyment of working on it.

The human must remain the primary author of the project. In particular,
architectural decisions, feature design, public APIs, domain modeling, and
significant implementation choices should be made by a human.

Self-contained implementations of external, well-defined formats or standards
used by codecs (like iso9660, or the whole 3DS codecs suite) may be delegated
more broadly to an LLM. This applies especially when porting existing algorithms
from another language, in which case proper attribution to the original author(s)
must be present in the codec's module-level rustdocs, and a mention must be added
in the `ATTRIBUTION.md` file.

However, even in those cases, no LLM may modify the existing architecture or
public traits to suit their needs.

Therefore:

* Do not independently design or implement new features unless explicitly asked.
* Do not make architectural, domain-model, or public-API decisions on the human's
  behalf. When such a decision is required and has not already been made, stop
  and discuss it first.
* Do not turn a design discussion into an implementation without being asked.
* Do not modify public APIs or break established contracts without explicit human
  approval.
* You may discuss architecture, design choices, APIs, and trade-offs.
* You may review human-written code and suggest improvements.
* You may provide small, focused examples when they help explain an idea.
* Prefer teaching and suggesting idiomatic Rust patterns when the human is
  actively implementing something themselves.
* You DO NOT have push privileges. The only place you're allowed to even commit
  is in non-master branches/worktrees you are working in, and the only commit
  message you're allowed to write is "WIP".
* Codec implementations must follow the skill `ludared-codec-authoring`.
* First inspect the existing code and architecture.
* Do not perform unrelated refactors or fix unrelated issues unless asked.
* Run the relevant tests, formatting, and linting after implementation.
* Report pre-existing or out-of-scope issues separately.
