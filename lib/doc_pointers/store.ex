defmodule DocPointers.Store do
  use GenServer

  alias DocPointers.Pointer

  def start_link(opts) do
    root = Keyword.fetch!(opts, :root)
    GenServer.start_link(__MODULE__, root, name: __MODULE__)
  end

  def set_root(root), do: GenServer.call(__MODULE__, {:set_root, root})
  def get(uuid), do: GenServer.call(__MODULE__, {:get, uuid})
  def get_by_token(token), do: GenServer.call(__MODULE__, {:get_by_token, token})
  def put(pointer), do: GenServer.call(__MODULE__, {:put, pointer})
  def update(uuid, updates), do: GenServer.call(__MODULE__, {:update, uuid, updates})
  def all, do: GenServer.call(__MODULE__, :all)
  def token_exists?(token), do: GenServer.call(__MODULE__, {:token_exists?, token})

  def list(opts \\ []) do
    GenServer.call(__MODULE__, {:list, opts})
  end

  # -- Server --

  @impl true
  def init(root) do
    state = %{root: root, pointers: %{}, token_index: %{}}
    state = load_pointers(state)
    {:ok, state}
  end

  @impl true
  def handle_call({:set_root, root}, _from, state) do
    state = %{state | root: root}
    state = load_pointers(state)
    {:reply, :ok, state}
  end

  def handle_call({:get, uuid}, _from, state) do
    {:reply, Map.get(state.pointers, uuid), state}
  end

  def handle_call({:get_by_token, token}, _from, state) do
    case Map.get(state.token_index, token) do
      nil -> {:reply, nil, state}
      uuid -> {:reply, Map.get(state.pointers, uuid), state}
    end
  end

  def handle_call({:put, %Pointer{} = pointer}, _from, state) do
    state = put_pointer(state, pointer)
    save_pointers(state)
    {:reply, :ok, state}
  end

  def handle_call({:update, uuid, updates}, _from, state) do
    case Map.get(state.pointers, uuid) do
      nil ->
        {:reply, {:error, :not_found}, state}

      pointer ->
        now = DateTime.utc_now() |> DateTime.to_iso8601()

        updated =
          pointer
          |> maybe_update(:description, updates)
          |> maybe_update(:class, updates)
          |> maybe_update(:line, updates)
          |> maybe_update(:file_path, updates)
          |> Map.put(:updated_at, now)

        state = put_pointer(state, updated)
        save_pointers(state)
        {:reply, {:ok, updated}, state}
    end
  end

  def handle_call(:all, _from, state) do
    {:reply, Map.values(state.pointers), state}
  end

  def handle_call({:token_exists?, token}, _from, state) do
    {:reply, Map.has_key?(state.token_index, token), state}
  end

  def handle_call({:list, opts}, _from, state) do
    pointers =
      state.pointers
      |> Map.values()
      |> maybe_filter_prefix(opts[:file_prefix])
      |> maybe_filter_class(opts[:class])
      |> Enum.sort_by(& &1.created_at)

    offset = opts[:offset] || 0
    limit = opts[:limit] || 50

    result = pointers |> Enum.drop(offset) |> Enum.take(limit)
    {:reply, {result, length(pointers)}, state}
  end

  # -- Internals --

  defp put_pointer(state, %Pointer{} = pointer) do
    %{
      state
      | pointers: Map.put(state.pointers, pointer.uuid, pointer),
        token_index: Map.put(state.token_index, pointer.token, pointer.uuid)
    }
  end

  defp maybe_update(pointer, field, updates) do
    case Map.get(updates, field) do
      nil -> pointer
      value -> Map.put(pointer, field, value)
    end
  end

  defp maybe_filter_prefix(pointers, nil), do: pointers

  defp maybe_filter_prefix(pointers, prefix) do
    Enum.filter(pointers, fn p -> p.file_path && String.starts_with?(p.file_path, prefix) end)
  end

  defp maybe_filter_class(pointers, nil), do: pointers

  defp maybe_filter_class(pointers, class) do
    Enum.filter(pointers, fn p -> p.class == class end)
  end

  defp meta_dir(state), do: Path.join(state.root, ".meta")
  defp pointers_path(state), do: Path.join(meta_dir(state), "pointers.yaml")
  defp legacy_json_path(state), do: Path.join([state.root, "docs", "doc-pointer-db.json"])

  defp load_pointers(state) do
    state = %{state | pointers: %{}, token_index: %{}}

    cond do
      File.exists?(pointers_path(state)) ->
        load_from_yaml(state)

      File.exists?(legacy_json_path(state)) ->
        state = import_legacy_json(state)
        save_pointers(state)
        state

      true ->
        state
    end
  end

  defp load_from_yaml(state) do
    case YamlElixir.read_from_file(pointers_path(state)) do
      {:ok, %{"pointers" => pointers}} when is_map(pointers) ->
        Enum.reduce(pointers, state, fn {uuid, data}, acc ->
          pointer = Pointer.from_map(uuid, data)
          put_pointer(acc, pointer)
        end)

      _ ->
        state
    end
  end

  defp import_legacy_json(state) do
    case File.read(legacy_json_path(state)) do
      {:ok, content} ->
        case Jason.decode(content) do
          {:ok, entries} when is_map(entries) ->
            Enum.reduce(entries, state, fn {token, data}, acc ->
              name = DocPointers.UUID5.build_name(data["name"] || token)
              uuid_bytes = DocPointers.UUID5.generate(name)
              uuid = DocPointers.UUID5.to_string(uuid_bytes)

              pointer =
                Pointer.new(%{
                  uuid: uuid,
                  token: token,
                  file_path: data["path"],
                  function: data["name"] || "unknown",
                  description: data["description"] || "",
                  line: data["line"]
                })

              put_pointer(acc, pointer)
            end)

          _ ->
            state
        end

      _ ->
        state
    end
  end

  defp save_pointers(state) do
    dir = meta_dir(state)
    File.mkdir_p!(dir)

    yaml_map =
      state.pointers
      |> Enum.sort_by(fn {uuid, _} -> uuid end)
      |> Enum.map(fn {uuid, pointer} -> {uuid, Pointer.to_map(pointer)} end)
      |> Map.new()

    content = Ymlr.document!(%{"pointers" => yaml_map})
    File.write!(pointers_path(state), content)
  end
end
