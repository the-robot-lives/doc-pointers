defmodule DocPointers.MCP.Tools.GenerateBatch do
  use Noizu.MCP.Server.Tool,
    name: "doc-pointer/generate-batch",
    description: """
    Register multiple doc-pointers in one call. Each entry provides
    {name, type, location, description}; `name` drives UUID derivation and is
    recorded as the pointer function, `type` (e.g. "function", "module") is
    stored as the pointer class.

    `location` may be an absolute path or a path relative to the project root.
    It is verified on disk and normalized before storage, so metadata lands in
    the correct (submodule-aware) .meta/pointers.yaml.

    Returns one record per entry: {name, uuid, token, marker, location, status}
    plus a `failed` list for entries that could not be registered.
    """,
    annotations: [destructive_hint: true]

  input do
    field(:entries, {:array, :object}, required: true) do
      field(:name, :string,
        required: true,
        description:
          "Pointer name — drives UUID derivation and is recorded as the pointer function"
      )

      field(:type, :string,
        description: "Entity type (e.g. function, module) — stored as the pointer class"
      )

      field(:location, :string,
        required: true,
        description:
          "Absolute path or path relative to project root; verified on disk and normalized " <>
            "so metadata lands in the correct (submodule-aware) .meta/pointers.yaml"
      )

      field(:description, :string,
        description: "Human-readable description of the code location"
      )
    end

    field(:confirm, :boolean,
      description: "Required true unless the server was started with --write"
    )
  end

  @impl true
  def call(args, ctx) do
    with :ok <- DocPointers.MCP.Writes.authorize(args, ctx) do
      do_call(args)
    end
  end

  defp do_call(args) do
    entries = args[:entries] || args["entries"]

    if is_list(entries) and entries != [] do
      root = DocPointers.Store.root()

      {registered, failed} =
        entries
        |> Enum.with_index()
        |> Enum.reduce({[], []}, fn {entry, idx}, {ok, bad} ->
          case process_entry(entry, idx, root) do
            {:ok, result} -> {[result | ok], bad}
            {:error, reason} -> {ok, [reason | bad]}
          end
        end)

      {:ok,
       %{
         registered: Enum.reverse(registered),
         failed: Enum.reverse(failed),
         total: length(entries),
         count: length(registered)
       }}
    else
      {:error, "entries must be a non-empty array of {name, type, location, description} objects"}
    end
  end

  defp process_entry(entry, idx, root) when is_map(entry) do
    name = fetch(entry, :name)
    location = fetch(entry, :location)

    cond do
      blank?(name) ->
        {:error, entry_error(idx, name, "missing required field: name")}

      blank?(location) ->
        {:error, entry_error(idx, name, "missing required field: location")}

      true ->
        register_entry(entry, idx, name, location, root)
    end
  end

  defp process_entry(_entry, idx, _root) do
    {:error, entry_error(idx, nil, "entry must be an object")}
  end

  defp register_entry(entry, idx, name, location, root) do
    with {:ok, rel_path} <- normalize_location(location, root) do
      case DocPointers.MCP.Tools.Generate.register(%{
             base_name: name,
             file_path: rel_path,
             class: fetch(entry, :type),
             function: name,
             description: fetch(entry, :description)
           }) do
        {:ok, result} ->
          {:ok,
           result
           |> Map.merge(%{name: name, type: fetch(entry, :type), location: rel_path})
           |> Map.put(:status, :ok)}

        {:error, reason} ->
          {:error, entry_error(idx, name, reason)}
      end
    else
      {:error, reason} -> {:error, entry_error(idx, name, reason)}
    end
  end

  # Absolute paths must exist and live under the project root; they are
  # relativized so Store.put/1 can place metadata in the owning
  # submodule's .meta/pointers.yaml. Relative paths are verified against root.
  defp normalize_location(location, root) when is_binary(location) do
    path = Path.expand(location, root)

    if File.exists?(path) do
      if Path.type(path) == :absolute do
        case Path.relative_to(path, root) do
          ^path -> {:error, "path is outside the project root (#{root}): #{location}"}
          rel -> {:ok, rel}
        end
      else
        {:ok, location}
      end
    else
      {:error, "file not found: #{location}"}
    end
  end

  defp normalize_location(_location, _root) do
    {:error, "location must be a string path"}
  end

  defp fetch(entry, key) when is_map(entry) do
    entry[key] || entry[Atom.to_string(key)]
  end

  defp blank?(value), do: is_nil(value) or value == ""

  defp entry_error(idx, name, reason) do
    %{index: idx, name: name, status: :error, error: reason}
  end
end
