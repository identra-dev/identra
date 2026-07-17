# identra-mcp

MCP servers exposed as thin adapters over `identra-core`. Server side only: just the transport
the agent CLIs actually speak, hand-rolled over `axum`.

The context bus lets two agent nodes share context, but only when an edge joins them:
`list_peers`, `get_peer_context`, and `send_to_node`, each gated on that edge, bound to loopback
with a per-node bearer token. The edge is the authorization: no edge, no context, no message.
