defmodule DocPointers.MCP.Runtime do
  @moduledoc false

  @default_port 4242
  @loopback {127, 0, 0, 1}

  def parse(args) do
    {opts, _rest, _invalid} =
      OptionParser.parse(args,
        strict: [root: :string, port: :integer, write: :boolean],
        aliases: [w: :write]
      )

    opts
  end

  # Mix tasks must select the root before app.start initializes the Store.
  # Changing it afterwards repeats the monorepo scan and can time out on a
  # large checkout. The command-line option takes precedence over the env var.
  def boot!(opts) do
    if root = opts[:root] do
      System.put_env("DOC_POINTERS_ROOT", root)
    end

    Mix.Task.run("app.start")
    configure!(Keyword.delete(opts, :root))
  end

  def configure!(opts) do
    if root = opts[:root] do
      DocPointers.Store.set_root(root)
    end

    Application.put_env(:doc_pointers, :mcp_writes, writes_enabled?(opts))
    :ok
  end

  def writes_enabled?(opts) do
    opts[:write] == true or env_flag?("DOC_POINTERS_MCP_WRITES")
  end

  def env_flag?(name) do
    System.get_env(name) in ["1", "true", "TRUE", "yes", "YES"]
  end

  def port(opts) do
    cond do
      is_integer(opts[:port]) -> opts[:port]
      p = System.get_env("DOC_POINTERS_PORT") -> String.to_integer(p)
      true -> @default_port
    end
  end

  def start_stdio! do
    Logger.configure(level: :warning)
    force_byte_mode_stdio()

    {:ok, _pid} =
      Supervisor.start_link([{DocPointers.MCP, transport: :stdio}],
        strategy: :one_for_one,
        name: DocPointers.MCP.Supervisor
      )

    :ok
  end

  # JSON-RPC is UTF-8 on the wire, and Jason owns all encoding. The BEAM's
  # standard-io device, however, follows the host locale: under a UTF-8
  # locale, IO.binwrite re-encodes latin1-interpreted binaries (hieroglyph
  # tokens leave double-encoded) and IO.binread stalls on multibyte lines
  # (token-keyed lookups hang until the client times out). Forcing the device
  # into binary + latin1 makes both directions raw byte passthrough; the JSON
  # layer never sees the difference. Idempotent; failures are non-fatal
  # (e.g. no group leader under some test harnesses).
  defp force_byte_mode_stdio do
    try do
      :io.setopts(:standard_io, [{:binary, true}, {:encoding, :latin1}])
      :ok
    rescue
      _ -> :ok
    catch
      _, _ -> :ok
    end
  end

  def start_http!(port) do
    children = [
      DocPointers.MCP,
      {Bandit,
       plug: {Noizu.MCP.Transport.StreamableHTTP.Plug, server: DocPointers.MCP},
       scheme: :http,
       port: port,
       ip: @loopback}
    ]

    {:ok, _pid} =
      Supervisor.start_link(children,
        strategy: :one_for_one,
        name: DocPointers.MCP.Supervisor
      )

    :ok
  end
end
