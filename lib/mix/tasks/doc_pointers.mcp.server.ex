defmodule Mix.Tasks.DocPointers.Mcp.Server do
  @shortdoc "Run the doc-pointers MCP server over HTTP"
  @moduledoc """
  Starts the doc-pointers MCP server on Streamable HTTP.

  ## Usage

      mix doc_pointers.mcp.server
      mix doc_pointers.mcp.server --port 4242 --root /path/to/project

  ## Options

    * `--port` - HTTP port (default 4242, or DOC_POINTERS_PORT env)
    * `--root` - Project root for .meta/ storage (default DOC_POINTERS_ROOT env or cwd)

  ## Claude Code registration

      claude mcp add doc-pointers --transport http http://localhost:4242/mcp
  """
  use Mix.Task

  @default_port 4242
  @requirements ["app.start"]

  @impl Mix.Task
  def run(args) do
    {opts, _rest, _invalid} =
      OptionParser.parse(args, strict: [root: :string, port: :integer])

    if root = opts[:root] do
      DocPointers.Store.set_root(root)
    end

    port =
      opts[:port] ||
        case System.get_env("DOC_POINTERS_PORT") do
          nil -> @default_port
          p -> String.to_integer(p)
        end

    children = [
      DocPointers.MCP,
      {Bandit,
       plug: {Noizu.MCP.Transport.StreamableHTTP.Plug, server: DocPointers.MCP},
       port: port,
       scheme: :http}
    ]

    {:ok, _pid} =
      Supervisor.start_link(children,
        strategy: :one_for_one,
        name: DocPointers.MCP.Supervisor
      )

    Mix.shell().info("doc-pointers MCP server running → http://localhost:#{port}/mcp")
    Process.sleep(:infinity)
  end
end
