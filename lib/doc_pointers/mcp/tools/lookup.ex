defmodule DocPointers.MCP.Tools.Lookup do
  use Noizu.MCP.Server.Tool,
    name: "doc-pointer/lookup",
    description:
      "Look up pointer metadata and bounded source snippets by UUID, typed marker, or legacy token.",
    annotations: [read_only_hint: true]

  alias DocPointers.{Marker, Store}

  input do
    field(:token, :string, description: "4-glyph hieroglyph token")
    field(:uuid, :string, description: "UUID, emoji:UUID, or emoji:token")

    field(:marker, :string,
      description: "Marker 〚emoji:TOKEN〛 or 〚emoji:UUID〛; legacy glyph marker accepted"
    )

    field(:file_path, :string, description: "Root-relative source path")
    field(:function_name, :string, description: "Function/name to search for")
  end

  @impl true
  def call(args, _ctx) do
    snapshot = Store.snapshot()

    with {:ok, selector} <- selector(args) do
      records = Enum.filter(snapshot.records, &matches?(&1, selector))
      results = Enum.map(records, &with_snippets(&1, snapshot.root))
      {:ok, %{results: results, count: length(results)}}
    end
  end

  defp selector(args) do
    cond do
      args[:uuid] ->
        parse_uuid_selector(args.uuid)

      args[:token] ->
        {:ok, {:token, args.token}}

      args[:marker] ->
        parse_marker_selector(args.marker)

      args[:file_path] || args[:function_name] ->
        {:ok, {:search, args[:file_path], args[:function_name]}}

      true ->
        {:error,
         "At least one search field (token, uuid, marker, file_path, function_name) is required"}
    end
  end

  defp parse_uuid_selector(input) do
    marker = if String.starts_with?(input, "〚"), do: input, else: "〚" <> input <> "〛"

    case Marker.parse(marker) do
      {:ok, %{uuid: uuid, kind: kind}} -> {:ok, {:typed, uuid, kind}}
      {:ok, %{token: token}} -> {:ok, {:token, token}}
      _ -> {:ok, {:uuid, input}}
    end
  end

  defp parse_marker_selector(input) do
    case Marker.parse(input) do
      {:ok, %{uuid: uuid, kind: kind}} ->
        {:ok, {:typed, uuid, kind}}

      {:ok, %{token: token}} ->
        {:ok, {:token, token}}

      _ ->
        case Marker.parse_legacy(input) do
          {:ok, %{token: token}} -> {:ok, {:token, token}}
          _ -> {:error, "Invalid doc-pointer marker"}
        end
    end
  end

  defp matches?(record, {:uuid, uuid}), do: record["uuid"] == uuid

  defp matches?(record, {:typed, uuid, kind}),
    do: record["uuid"] == uuid and record["kind"] == kind

  defp matches?(record, {:token, token}), do: record["token"] == token

  defp matches?(record, {:search, path, name}) do
    (is_nil(path) or record["file_path"] == path or
       Enum.any?(record["locations"] || [], &(&1["file_path"] == path))) and
      (is_nil(name) or record["function"] == name)
  end

  defp with_snippets(record, root) do
    locations =
      case record["locations"] || [] do
        [] -> [%{"file_path" => record["file_path"], "line" => record["line"]}]
        many -> many
      end

    record
    |> Map.put("marker", Marker.open(record["uuid"], record["kind"]))
    |> Map.put("snippets", Enum.map(locations, &snippet(root, &1)))
  end

  defp snippet(root, location) do
    path = location["file_path"]

    with true <- safe_relative_path?(path),
         absolute = Path.expand(path, root),
         true <- String.starts_with?(absolute, Path.expand(root) <> "/"),
         false <- symlink_in_path?(root, path),
         {:ok, content} <- File.read(absolute) do
      lines = String.split(content, ~r/\r?\n/, trim: false)
      line = location["line"] || 1
      end_line = location["end_line"] || line
      first = max(line - 2, 1)
      last = min(max(end_line + 2, first + 9), first + 39)

      location
      |> Map.put("start_line", first)
      |> Map.put("content", lines |> Enum.slice((first - 1)..(last - 1)) |> Enum.join("\n"))
    else
      false -> Map.put(location, "error", "source path is outside the root")
      true -> Map.put(location, "error", "source path is outside the root")
      {:error, reason} -> Map.put(location, "error", "source unavailable: #{reason}")
    end
  end

  defp safe_relative_path?(path),
    do:
      is_binary(path) and path != "" and Path.type(path) == :relative and
        not Enum.member?(Path.split(path), "..")

  defp symlink_in_path?(root, relative) do
    relative
    |> Path.split()
    |> Enum.scan(root, &Path.join(&2, &1))
    |> Enum.any?(fn path ->
      case File.lstat(path) do
        {:ok, %File.Stat{type: :symlink}} -> true
        _ -> false
      end
    end)
  end
end
