defmodule DocPointers.MCP do
  use Noizu.MCP.Server,
    name: "doc_pointers",
    version: "0.1.0",
    instructions: """
    Generate and manage doc-pointer hieroglyphic codes. Doc pointers are durable,
    code-stable cross-document references using UUIDv5-derived 4-character hieroglyphic
    tokens from Egyptian, Meroitic, and Anatolian Unicode blocks.

    Default tools are read-only: doc-pointer/lookup and doc-pointer/list.
    doc-pointer/generate and doc-pointer/update persist to .meta/pointers.yaml.
    They are listed when the server is started with --write (or DOC_POINTERS_MCP_WRITES=1);
    otherwise they require confirm=true (or a client confirmation prompt).

    Stored kinds are canonical strings: file, module, class, struct, interface,
    protocol, behaviour, function, logic, component, diagram. Source markers stay
    emoji (〚🔧:uuid〛); the emoji maps to a canonical default string when stored
    (📁→file, 📦→module, 🔌→interface, 🧩→component, 🔧→function, 🔀→logic, 📐→diagram).
    Tools accept string kinds and legacy emoji, and normalize emoji to strings.

    Pointers are keyed by full UUID in the .meta/pointers.yaml of the git repo that
    owns the file: the root for root-level files, each (nested) submodule's own
    folder for files inside it. Pass file_path relative to the server root; it is
    stored relative to the owning repo.
    """

  tool(DocPointers.MCP.Tools.Lookup, category: "Pointers")
  tool(DocPointers.MCP.Tools.List, category: "Pointers")
  tool(DocPointers.MCP.Tools.Generate, category: "Pointers", hidden: true)
  tool(DocPointers.MCP.Tools.Update, category: "Pointers", hidden: true)

  @impl true
  def handle_list_tools(cursor, _ctx) do
    Noizu.MCP.Server.Features.Tools.list_registered(
      __mcp__(:tools),
      cursor,
      include_hidden: DocPointers.MCP.Writes.enabled?()
    )
  end
end
