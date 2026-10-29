# Vision

> "Civilization advances by extending the number of important operations which we can perform without thinking about them." (Alfred North Whitehead)

## What Freenpipe is

Context infrastructure for AI agents: the ambient layer between people and their digital work.

The screen is the universal interface, about 10M bits per second and the highest-fidelity signal of what a person intends. During work it shows all the software people use, much of which AI is now absorbing. Freenpipe captures that context locally and makes it available to AI.

Not a memory tool. Not an assistant you prompt. An ambient automation layer that works in the background without prompting.

Freenpipe is an MIT fork of screenpipe at commit `892199f`. It keeps the base's direction and stays compatible with screenpipe's data, API, pipes and MCP configurations, without the upstream vendor's services. Beyond compatibility, Freenpipe designs its own features (see [docs/clean-room/README.md](docs/clean-room/README.md)).

## Why it exists

Every AI interaction today means stopping work, turning intent into a prompt, and waiting. That is a tax on every interaction. The screen already shows what you are doing; the context is right there.

Freenpipe builds the layer that gives AI the full context of digital work so it can act on its own. Recording plus AI makes it possible to reproduce digital work at high fidelity.

## Where this goes

1. **Now: memory.** Make desktop memory work so well that people rely on it. Record, Rewind, Ask: three verbs, nothing else.
2. **Next: context layer for AI agents.** An open API so any AI agent can query screen history, act on that context, and trigger without prompts.
3. **Later: everything.** Record everything you do, all day, safely, encrypted locally and connected to AI.

## Product principles

- **Stability over features.** Users who stay rely on it every day. Users who leave hit bugs. Fix what is broken before building what is new.
- **No feature creep.** Every feature must serve Record, Rewind or Ask. If it does not, it does not ship.
- **Respect the user's machine.** CPU, memory, disk: Freenpipe runs all day in the background. Performance is not optional. Target: under 20% CPU and under 3 GB RAM on release builds.

## Engineering principles

- **Ship daily.** Small, focused changes. Every commit should be deployable.
- **Simple, powerful abstractions.** Write the spec yourself, then use AI to execute it.
- **Local-first always.** Data never leaves the device unless the user explicitly opts in, for example sync or archive to storage the user chooses. Encryption is zero-knowledge.
- **No phoning home.** No telemetry and no vendor services. Cloud features use the user's own keys or self-hosted services.
- **Windows first, others kept working.** Windows must build and run. The macOS and Linux code paths must keep compiling.
- **Open source by default.** Trust is earned through transparency.

## Design voice

- State facts. No marketing language.
- No emoji in the product. No exclamation marks. Remove what is unnecessary.
- Black and white. 1px borders. Sharp corners. No shadows, no gradients.
- 40% of any composition should be empty space.
- When in doubt, remove.

## Measuring progress

Freenpipe collects no usage data, so it does not track usage metrics. Progress is measured against the plan ([docs/clean-room/STATUS.md](docs/clean-room/STATUS.md)), the regression checklist in [TESTING.md](TESTING.md), and resource use on release builds.

## What we believe

- Sending prompts is obsolete; AI should watch and work.
- People spend more than 8 hours a day on screens. Digital work is bottlenecked by human attention, and AI needs full context to automate it.
- Vision is the universal interface; it is the most powerful sense of most animals.
- People should be augmented by AI, and use the time it buys them to expand their understanding and explore.
- Positive-sum games. Give first. Optimise for the long term.
- Radical transparency. No ego. Idea meritocracy. Constructive disagreement.
- Truth over comfort. Bold over safe. Ship over plan.
