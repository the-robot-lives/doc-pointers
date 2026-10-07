defmodule Mix.Tasks.DocPointers.Mcp.Server do
  @shortdoc "Run the doc-pointers MCP server over loopback HTTP"
  @moduledoc """
  Starts the doc-pointers MCP server on Streamable HTTP bound to 127.0.0.1.

  Prefer `mix doc_pointers.mcp.stdio` for local MCP clients.

  ## Usage

      mix doc_pointers.mcp.server
      mix doc_pointers.mcp.server --port 4242 --root /path/to/project
      mix doc_pointers.mcp.server --write

  ## Options

    * `--port` - HTTP port (default 4242, or DOC_POINTERS_PORT env)
    * `--root` - Project root for .meta/ storage (default DOC_POINTERS_ROOT env or cwd)
    * `--write` - List and allow generate/update without `confirm=true`
      (or set DOC_POINTERS_MCP_WRITES=1)

  The listener is loopback-only (`127.0.0.1`). There is no auth.
  """
  use Mix.Task

  @requirements ["app.config"]

  @impl Mix.Task
  def run(args) do
    opts = DocPointers.MCP.Runtime.parse(args)
    DocPointers.MCP.Runtime.boot!(opts)
    port = DocPointers.MCP.Runtime.port(opts)
    DocPointers.MCP.Runtime.start_http!(port)

    DocPointers.MCP.ConnectionHelp.print_http!(port, opts)
    Process.sleep(:infinity)
  end
end
