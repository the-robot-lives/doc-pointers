defmodule Mix.Tasks.DocPointers.Cli do
  @shortdoc "Run the doc-pointers JSON-lines store backend"
  @moduledoc """
  Reads one JSON request per stdin line and writes one JSON response per stdout line.

      mix doc_pointers.cli --root /path/to/project

  Operations are `status` (or `list`) and `reconcile`. Reconcile accepts
  `records` and a boolean `write` flag; omission of `write` is a dry run.
  Diagnostics belong on stderr because stdout is the protocol stream.
  """
  use Mix.Task

  @requirements ["app.config"]
  @schema "pointers-yaml-v1"

  @impl Mix.Task
  def run(args) do
    opts = DocPointers.MCP.Runtime.parse(args)
    Application.put_env(:doc_pointers, :skip_legacy_import, true)
    DocPointers.MCP.Runtime.boot!(opts)
    :io.setopts(:standard_io, [{:binary, true}, {:encoding, :latin1}])

    IO.binstream(:stdio, :line)
    |> Enum.each(fn line ->
      response =
        try do
          line |> Jason.decode!() |> dispatch()
        rescue
          error -> %{ok: false, error: Exception.message(error)}
        end

      IO.binwrite(:stdio, Jason.encode!(response) <> "\n")
    end)
  end

  defp dispatch(%{"op" => op}) when op in ["status", "list"] do
    snapshot = DocPointers.Store.snapshot()
    %{ok: true, root: snapshot.root, schema: @schema, records: snapshot.records}
  end

  defp dispatch(%{"op" => "reconcile"} = request) do
    with :ok <- validate_write(request),
         {:ok, result} <-
           DocPointers.Store.reconcile(request["records"], request["write"] == true) do
      snapshot = DocPointers.Store.snapshot()

      %{
        ok: true,
        root: snapshot.root,
        schema: @schema,
        changed: result.changed,
        inserted: result.inserted,
        updated: result.updated,
        records: result.records
      }
    else
      {:error, reason} -> %{ok: false, error: reason}
    end
  end

  defp dispatch(_), do: %{ok: false, error: "unknown operation"}

  defp validate_write(request) do
    if Map.get(request, "write", false) in [true, false] do
      :ok
    else
      {:error, "write must be a boolean"}
    end
  end
end
