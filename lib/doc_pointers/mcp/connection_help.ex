defmodule DocPointers.MCP.ConnectionHelp do
  @moduledoc false

  def print_stdio!(opts) do
    root = root(opts)
    project = File.cwd!() |> Path.expand()
    script = ~S(cd "$1" && exec mix doc_pointers.mcp.stdio --root "$2")

    script =
      if DocPointers.MCP.Runtime.writes_enabled?(opts), do: script <> " --write", else: script

    command = "sh -c #{shell_quote(script)} sh #{shell_quote(project)} #{shell_quote(root)}"

    IO.puts(:stderr, "doc-pointers MCP ready on stdio (root: #{root})")
    IO.puts(:stderr, "Claude: claude mcp add doc-pointers -- #{command}")
    IO.puts(:stderr, "Codex:  codex mcp add doc-pointers -- #{command}")
  end

  def print_http!(port, opts) do
    url = "http://127.0.0.1:#{port}/"

    IO.puts(:stderr, "doc-pointers MCP ready at #{url} (root: #{root(opts)})")
    IO.puts(:stderr, "Claude: claude mcp add --transport http doc-pointers #{url}")
    IO.puts(:stderr, "Codex:  codex mcp add doc-pointers --url #{url}")
  end

  defp root(opts) do
    (opts[:root] || System.get_env("DOC_POINTERS_ROOT") ||
       Application.get_env(:doc_pointers, :root) || File.cwd!())
    |> Path.expand()
  end

  defp shell_quote(value), do: "'" <> String.replace(value, "'", "'\\''") <> "'"
end
