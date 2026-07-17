# identra-agents

Agent presets and fan-out orchestration recipes: mostly config and prompts, not a new engine.

There is no supervisor class. A preset is a named role (system prompt, which CLI, allowed tools);
an orchestrator agent fans work out to instances of them over the context-bus MCP tools. This is
where those presets and recipes live.

See `health-recipe.md` for the two-agent handoff that drives the bus tools end to end.
