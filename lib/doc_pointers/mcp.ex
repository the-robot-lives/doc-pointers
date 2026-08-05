defmodule DocPointers.MCP do
  use Noizu.MCP.Server,
    name: "doc_pointers",
    version: "0.1.0",
    instructions: """
    Generate and manage doc-pointer hieroglyphic codes. Doc pointers are durable,
    code-stable cross-document references using UUIDv5-derived 4-character hieroglyphic
    tokens from Egyptian, Meroitic, and Anatolian Unicode blocks.

    Use doc-pointer/generate to mint new pointers, doc-pointer/lookup to find existing
    ones, doc-pointer/list to browse, and doc-pointer/update to change metadata.

    All pointers are stored in .meta/pointers.yaml at the project root, keyed by full UUID.
    """

  tool DocPointers.MCP.Tools.Generate, category: "Pointers"
  tool DocPointers.MCP.Tools.Lookup, category: "Pointers"
  tool DocPointers.MCP.Tools.List, category: "Pointers"
  tool DocPointers.MCP.Tools.Update, category: "Pointers"
end
