defmodule DocPointers.MCP.ConnectionHelpTest do
  use ExUnit.Case, async: true

  import ExUnit.CaptureIO

  alias DocPointers.MCP.ConnectionHelp

  test "stdio launch prints usable client commands to stderr only" do
    root = "/tmp/Keith's Docs"

    stdout =
      capture_io(fn ->
        stderr =
          capture_io(:stderr, fn -> ConnectionHelp.print_stdio!(root: root, write: true) end)

        send(self(), {:stderr, stderr})
      end)

    assert stdout == ""
    assert_received {:stderr, stderr}
    assert stderr =~ "doc-pointers MCP ready on stdio"
    assert stderr =~ "Claude: claude mcp add doc-pointers -- sh -c"
    assert stderr =~ "Codex:  codex mcp add doc-pointers -- sh -c"
    assert stderr =~ ~S(exec mix doc_pointers.mcp.stdio --root "$2" --write)
    assert stderr =~ "'/tmp/Keith'\\''s Docs'"
  end

  test "HTTP launch prints the bound root endpoint and registration commands" do
    stderr = capture_io(:stderr, fn -> ConnectionHelp.print_http!(4545, root: "/tmp/docs") end)

    assert stderr =~ "doc-pointers MCP ready at http://127.0.0.1:4545/"
    assert stderr =~ "claude mcp add --transport http doc-pointers http://127.0.0.1:4545/"
    assert stderr =~ "codex mcp add doc-pointers --url http://127.0.0.1:4545/"
    refute stderr =~ "/mcp"
  end
end
