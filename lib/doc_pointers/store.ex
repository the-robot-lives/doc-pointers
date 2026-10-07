defmodule DocPointers.Store do
  use GenServer

  alias DocPointers.Pointer

  defmodule InvalidStoreError do
    defexception [:message]
  end

  def start_link(opts) do
    root = Keyword.fetch!(opts, :root)
    GenServer.start_link(__MODULE__, root, name: __MODULE__)
  end

  # Monorepo-scale recursive scans can take well over the default 5s.
  @scan_timeout 120_000

  def set_root(root), do: GenServer.call(__MODULE__, {:set_root, root}, @scan_timeout)
  def get(uuid), do: GenServer.call(__MODULE__, {:get, uuid})
  def get_by_token(token), do: GenServer.call(__MODULE__, {:get_by_token, token})
  def put(pointer), do: GenServer.call(__MODULE__, {:put, pointer})
  def migrate, do: GenServer.call(__MODULE__, :migrate, @scan_timeout)
  def update(uuid, updates), do: GenServer.call(__MODULE__, {:update, uuid, updates})
  def all, do: GenServer.call(__MODULE__, :all)
  def token_exists?(token), do: GenServer.call(__MODULE__, {:token_exists?, token})
  def snapshot, do: GenServer.call(__MODULE__, :snapshot, @scan_timeout)

  def reconcile(records, write? \\ false),
    do: GenServer.call(__MODULE__, {:reconcile, records, write?}, @scan_timeout)

  def list(opts \\ []) do
    GenServer.call(__MODULE__, {:list, opts})
  end

  # -- Server --

  @impl true
  def init(root) do
    root = Path.expand(root)
    submodules = detect_submodules(root)

    state = %{
      root: root,
      submodules: submodules,
      pointers: %{},
      token_index: %{},
      store_membership: %{},
      fingerprint: nil
    }

    state = state |> reload() |> maybe_load_legacy_locked()
    {:ok, state}
  end

  @impl true
  def handle_call({:set_root, root}, _from, state) do
    root = Path.expand(root)
    submodules = detect_submodules(root)

    state = %{
      state
      | root: root,
        submodules: submodules,
        pointers: %{},
        token_index: %{},
        store_membership: %{},
        fingerprint: nil
    }

    state = state |> reload() |> maybe_load_legacy_locked()
    {:reply, :ok, state}
  end

  def handle_call({:get, uuid}, _from, state) do
    state = refresh(state)
    {:reply, Map.get(state.pointers, uuid), state}
  end

  def handle_call({:get_by_token, token}, _from, state) do
    state = refresh(state)

    case Map.get(state.token_index, token) do
      nil -> {:reply, nil, state}
      uuid -> {:reply, Map.get(state.pointers, uuid), state}
    end
  end

  def handle_call({:put, %Pointer{} = pointer}, _from, state) do
    with_lock(state, fn state ->
      {store_key, adjusted} = resolve_and_adjust(state, pointer)
      state = put_pointer(state, adjusted, store_key)
      save_store(state, store_key)
      {:reply, :ok, %{state | fingerprint: store_fingerprint(state)}}
    end)
  end

  def handle_call({:update, uuid, updates}, _from, state) do
    with_lock(state, fn state -> do_update(uuid, updates, state) end)
  end

  def handle_call(:all, _from, state) do
    state = refresh(state)
    {:reply, Map.values(state.pointers), state}
  end

  def handle_call(:migrate, _from, state) do
    with_lock(state, &do_migrate/1)
  end

  def handle_call({:token_exists?, token}, _from, state) do
    state = refresh(state)
    {:reply, Map.has_key?(state.token_index, token), state}
  end

  def handle_call({:list, opts}, _from, state) do
    state = refresh(state)

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

  def handle_call({:reconcile, records, write?}, _from, state) do
    if write? do
      with_lock(state, fn state -> do_reconcile(records, true, state) end)
    else
      do_reconcile(records, false, refresh(state))
    end
  end

  def handle_call(:snapshot, _from, state) do
    state = refresh(state)
    {:reply, %{root: Path.expand(state.root), records: public_records(state)}, state}
  end

  defp do_update(uuid, updates, state) do
    case Map.get(state.pointers, uuid) do
      nil ->
        {:reply, {:error, :not_found}, state}

      pointer ->
        now = DateTime.utc_now() |> DateTime.to_iso8601()
        old_store_key = Map.get(state.store_membership, uuid, "")

        updated =
          pointer
          |> maybe_update(:description, updates)
          |> maybe_update(:class, updates)
          |> maybe_update(:line, updates)
          |> maybe_update(:file_path, updates)
          |> maybe_update(:kind, updates)
          |> then(fn p ->
            if Map.has_key?(updates, :kind), do: %{p | kind_explicit: true}, else: p
          end)
          |> Map.put(:updated_at, now)

        # file_path is stored relative to its store; re-prefix before re-resolving
        # so the pointer migrates to the owning submodule when it moves.
        {new_store_key, adjusted} =
          resolve_and_adjust(state, %{
            updated
            | file_path: prefix_path(old_store_key, updated.file_path)
          })

        state = put_pointer(state, adjusted, new_store_key)
        save_store(state, new_store_key)

        if new_store_key != old_store_key do
          save_store(state, old_store_key)
        end

        {:reply, {:ok, adjusted}, %{state | fingerprint: store_fingerprint(state)}}
    end
  end

  defp do_migrate(state) do
    {state, moved, touched} =
      state.pointers
      |> Map.values()
      |> Enum.reduce({state, 0, MapSet.new()}, fn pointer, {st, moved, touched} ->
        old_store_key = Map.get(st.store_membership, pointer.uuid, "")
        root_relative = prefix_path(old_store_key, pointer.file_path)
        {new_store_key, adjusted} = resolve_and_adjust(st, %{pointer | file_path: root_relative})

        if new_store_key == old_store_key do
          {st, moved, touched}
        else
          st = put_pointer(st, adjusted, new_store_key)
          {st, moved + 1, touched |> MapSet.put(old_store_key) |> MapSet.put(new_store_key)}
        end
      end)

    Enum.each(MapSet.to_list(touched), &save_store(state, &1))

    {:reply, %{moved: moved, stores: MapSet.to_list(touched)},
     %{state | fingerprint: store_fingerprint(state)}}
  end

  defp do_reconcile(records, write?, state) when is_list(records) do
    result =
      Enum.reduce_while(records, {state, MapSet.new(), 0, 0}, fn attrs,
                                                                 {current, touched, inserted,
                                                                  updated} ->
        case reconcile_one(attrs, current) do
          {:ok, next, old_key, new_key, change} ->
            touched =
              if change == :unchanged,
                do: touched,
                else: touched |> MapSet.put(old_key) |> MapSet.put(new_key)

            {:cont,
             {next, touched, inserted + if(change == :inserted, do: 1, else: 0),
              updated + if(change == :updated, do: 1, else: 0)}}

          {:error, reason} ->
            {:halt, {:error, reason}}
        end
      end)

    case result do
      {:error, reason} ->
        {:reply, {:error, reason}, state}

      {next, touched, inserted, updated} ->
        if write? do
          Enum.each(touched, &save_store(next, &1))
        end

        reply = %{
          changed: MapSet.size(touched) > 0,
          inserted: inserted,
          updated: updated,
          records: public_records(next)
        }

        persisted = if write?, do: %{next | fingerprint: store_fingerprint(next)}, else: state
        {:reply, {:ok, reply}, persisted}
    end
  end

  defp do_reconcile(_records, _write?, state),
    do: {:reply, {:error, "records must be an array"}, state}

  defp reconcile_one(attrs, state) when is_map(attrs) do
    requested_uuid = attrs["uuid"]
    existing_by_uuid = state.pointers[requested_uuid]

    token =
      attrs["token"] ||
        (existing_by_uuid && existing_by_uuid.token) ||
        token_from_uuid(requested_uuid)

    file_path = attrs["file_path"]
    function = attrs["function"]
    description = attrs["description"]
    kind = attrs["kind"] || (existing_by_uuid && existing_by_uuid.kind) || "🔧"
    owner = state.store_membership[requested_uuid] || ""

    locations =
      attrs["locations"] ||
        (existing_by_uuid && public_locations(existing_by_uuid.locations, owner)) || []

    attrs =
      attrs |> Map.put("token", token) |> Map.put("kind", kind) |> Map.put("locations", locations)

    cond do
      not (is_binary(token) and String.length(token) == 4) ->
        {:error, "record token must contain exactly four glyphs"}

      not DocPointers.Marker.valid_kind?(kind) ->
        {:error, "record kind must be one of 📁, 📦, 🔌, 🧩, 🔧, 🔀, 📐"}

      not valid_locations?(locations) ->
        {:error,
         "record locations must contain root-relative file_path and optional line/end_line"}

      kind != "🧩" and length(locations) > 1 ->
        {:error, "only component records may have multiple locations"}

      kind != "🧩" and locations != [] and hd(locations)["file_path"] != file_path ->
        {:error, "single-anchor location must match record file_path"}

      not valid_relative_path?(file_path) ->
        {:error, "record file_path must be relative to the root"}

      not (is_binary(function) and function != "") ->
        {:error, "record function is required"}

      not is_binary(description) ->
        {:error, "record description is required"}

      not (is_nil(requested_uuid) or is_binary(requested_uuid)) ->
        {:error, "record uuid must be a string"}

      true ->
        reconcile_valid(attrs, state)
    end
  end

  defp reconcile_one(_attrs, _state), do: {:error, "each record must be an object"}

  defp reconcile_valid(attrs, state) do
    attrs =
      Map.update!(attrs, "locations", fn locations ->
        Enum.sort_by(locations, fn location ->
          {location["file_path"], location["line"] || 0, location["end_line"] || 0}
        end)
      end)

    attrs =
      if attrs["kind"] == "🧩" and attrs["locations"] != [] do
        first = hd(attrs["locations"])
        attrs |> Map.put("file_path", first["file_path"]) |> Map.put("line", first["line"])
      else
        attrs
      end

    token = attrs["token"]
    existing_uuid = state.token_index[token]
    requested_uuid = attrs["uuid"]

    uuid =
      existing_uuid || requested_uuid ||
        DocPointers.UUID5.build_name(attrs["function"])
        |> DocPointers.UUID5.generate()
        |> DocPointers.UUID5.to_string()

    cond do
      existing_uuid && requested_uuid && existing_uuid != requested_uuid ->
        {:error, "token #{token} belongs to a different UUID"}

      state.pointers[uuid] && state.pointers[uuid].token != token ->
        {:error, "UUID #{uuid} belongs to a different token"}

      true ->
        existing = state.pointers[uuid]
        old_key = state.store_membership[uuid] || ""

        pointer =
          if existing do
            %{
              existing
              | file_path: attrs["file_path"],
                function: attrs["function"],
                description: attrs["description"],
                kind: attrs["kind"],
                kind_explicit: true,
                locations: attrs["locations"],
                line: Map.get(attrs, "line", existing.line),
                class: Map.get(attrs, "class", existing.class)
            }
          else
            Pointer.new(%{
              uuid: uuid,
              token: token,
              file_path: attrs["file_path"],
              function: attrs["function"],
              description: attrs["description"],
              kind: attrs["kind"],
              locations: attrs["locations"],
              line: attrs["line"],
              class: attrs["class"]
            })
          end

        {new_key, adjusted} = resolve_and_adjust(state, pointer)

        change =
          cond do
            is_nil(existing) -> :inserted
            existing == adjusted and old_key == new_key -> :unchanged
            true -> :updated
          end

        adjusted =
          if change == :updated,
            do: %{adjusted | updated_at: DateTime.utc_now() |> DateTime.to_iso8601()},
            else: adjusted

        {:ok, put_pointer(state, adjusted, new_key), old_key, new_key, change}
    end
  end

  defp public_records(state) do
    state.pointers
    |> Enum.map(fn {uuid, pointer} ->
      owner = state.store_membership[uuid] || ""
      file_path = if pointer.file_path, do: prefix_path(owner, pointer.file_path), else: nil

      pointer
      |> Pointer.to_map()
      |> Map.put("uuid", uuid)
      |> Map.put("kind", pointer.kind)
      |> Map.put("file_path", file_path)
      |> Map.put("locations", public_locations(pointer.locations, owner))
    end)
    |> Enum.sort_by(& &1["uuid"])
  end

  defp token_from_uuid(uuid) when is_binary(uuid) do
    if DocPointers.Marker.valid_uuid?(uuid) do
      uuid |> DocPointers.UUID5.from_string() |> DocPointers.Hieroglyph.encode()
    end
  end

  defp token_from_uuid(_), do: nil

  defp valid_locations?(locations) when is_list(locations) do
    Enum.all?(locations, fn
      %{"file_path" => path} = location ->
        valid_relative_path?(path) and
          valid_line?(Map.get(location, "line")) and
          valid_line?(Map.get(location, "end_line")) and
          (is_nil(location["end_line"]) or is_nil(location["line"]) or
             location["end_line"] >= location["line"])

      _ ->
        false
    end)
  end

  defp valid_locations?(_), do: false

  defp valid_line?(nil), do: true
  defp valid_line?(line), do: is_integer(line) and line > 0

  defp valid_relative_path?(path),
    do:
      is_binary(path) and path != "" and Path.type(path) == :relative and
        not Enum.member?(Path.split(path), "..")

  defp public_locations(locations, owner) do
    Enum.map(locations || [], fn location ->
      Map.update!(location, "file_path", &prefix_path(owner, &1))
    end)
  end

  # -- Store detection --
  #
  # A store is any git repo at or below the root: the root itself (store_key "")
  # plus every nested checkout (submodule) found via its `.git` entry. Routing by
  # git boundary (not .gitmodules) so nested submodules like Portfolio/Libs/ai/genai
  # resolve to their own folder rather than their top-level parent.

  @ignored_segments [".git", "_build", "deps", "node_modules", ".claude", "cover", "tmp"]

  # Walk Git's index for gitlinks instead of spawning a shell for every
  # `git submodule foreach` level. The latter takes minutes in large nested
  # monorepos. Only initialized submodules have a .git entry and a store.
  defp detect_submodules(root) do
    case gitlinks(root) do
      {:ok, links} ->
        links
        |> Enum.flat_map(fn link -> collect_gitlink(root, link) end)
        |> Enum.sort_by(&byte_size/1, :desc)

      :error ->
        fallback_detect_submodules(root)
    end
  end

  defp collect_gitlink(root, relative) do
    if ignored_dir?(relative) do
      []
    else
      path = Path.join(root, relative)

      if File.exists?(Path.join(path, ".git")) do
        nested =
          case gitlinks(path) do
            {:ok, links} -> Enum.flat_map(links, &collect_gitlink(root, Path.join(relative, &1)))
            :error -> []
          end

        [relative | nested]
      else
        []
      end
    end
  end

  defp gitlinks(path) do
    case System.cmd("git", ["ls-files", "--stage", "-z"], cd: path, stderr_to_stdout: true) do
      {out, 0} ->
        links =
          out
          |> :binary.split(<<0>>, [:global])
          |> Enum.flat_map(fn
            <<"160000 ", _::binary>> = entry ->
              case :binary.split(entry, <<9>>) do
                [_meta, name] -> [name]
                _ -> []
              end

            _ ->
              []
          end)

        {:ok, links}

      _ ->
        :error
    end
  end

  # Fallback for roots that are not a git superproject: bounded filesystem sweep.
  defp fallback_detect_submodules(root) do
    root
    |> Path.join("**/.git")
    |> Path.wildcard(match_dot: true)
    |> Enum.map(fn git_path ->
      git_path |> String.trim_trailing(".git") |> Path.relative_to(root)
    end)
    # Judge ignored segments on the root-relative path: a root under /tmp
    # (Linux tmp_dir) must not ignore every nested store.
    |> Enum.reject(&(&1 == "." or ignored_dir?(&1)))
    |> Enum.sort_by(&byte_size/1, :desc)
  end

  defp ignored_dir?(path) do
    path
    |> Path.split()
    |> Enum.any?(&(&1 in @ignored_segments))
  end

  defp resolve_store_key(submodules, file_path) when is_binary(file_path) do
    Enum.find(submodules, "", fn sub_path ->
      String.starts_with?(file_path, sub_path <> "/")
    end)
  end

  defp resolve_store_key(_submodules, _), do: ""

  defp resolve_and_adjust(state, %Pointer{} = pointer) do
    store_key =
      if pointer.kind == "🧩", do: "", else: resolve_store_key(state.submodules, pointer.file_path)

    adjusted =
      if store_key != "" and pointer.file_path do
        prefix = store_key <> "/"

        %{
          pointer
          | file_path: String.replace_prefix(pointer.file_path, prefix, ""),
            locations:
              Enum.map(pointer.locations || [], fn location ->
                Map.update!(location, "file_path", &String.replace_prefix(&1, prefix, ""))
              end)
        }
      else
        pointer
      end

    {store_key, adjusted}
  end

  defp prefix_path(_store_key, nil), do: nil
  defp prefix_path("", file_path), do: file_path
  defp prefix_path(store_key, file_path), do: Path.join(store_key, file_path)

  # -- Internals --

  defp put_pointer(state, %Pointer{} = pointer, store_key) do
    %{
      state
      | pointers: Map.put(state.pointers, pointer.uuid, pointer),
        token_index: Map.put(state.token_index, pointer.token, pointer.uuid),
        store_membership: Map.put(state.store_membership, pointer.uuid, store_key)
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

  defp store_root(state, ""), do: state.root
  defp store_root(state, store_key), do: Path.join(state.root, store_key)

  defp meta_dir(state, store_key), do: Path.join(store_root(state, store_key), ".meta")
  defp pointers_path(state, store_key), do: Path.join(meta_dir(state, store_key), "pointers.yaml")
  defp legacy_json_path(state), do: Path.join([state.root, "docs", "doc-pointer-db.json"])

  defp store_paths(state), do: Enum.map(["" | state.submodules], &pointers_path(state, &1))

  defp store_fingerprint(state) do
    Map.new(store_paths(state), fn path ->
      value =
        case File.stat(path, time: :native) do
          {:ok, stat} -> {stat.inode, stat.size, stat.mtime}
          _ -> nil
        end

      {path, value}
    end)
  end

  # Another process may replace YAML between requests. Always reload on reads;
  # the known store paths are reused, so this does not repeat gitlink discovery.
  defp refresh(state), do: reload(state)

  defp reload(state) do
    state
    |> Map.merge(%{pointers: %{}, token_index: %{}, store_membership: %{}})
    |> load_all_pointers()
    |> then(fn loaded -> %{loaded | fingerprint: store_fingerprint(loaded)} end)
  end

  # A lock directory is an atomic cross-process claim on the target root.
  # Never break a timed-out lock automatically: its owner may still be writing.
  defp with_lock(state, operation) do
    lock = Path.join(state.root, ".meta/.pointers.lock")
    File.mkdir_p!(Path.dirname(lock))
    acquire_lock!(lock, System.monotonic_time(:millisecond) + 30_000)

    try do
      operation.(reload(state))
    rescue
      error in InvalidStoreError ->
        {:reply, {:error, Exception.message(error)}, state}
    after
      File.rmdir(lock)
    end
  end

  defp acquire_lock!(lock, deadline) do
    case File.mkdir(lock) do
      :ok ->
        :ok

      {:error, :eexist} ->
        if System.monotonic_time(:millisecond) >= deadline do
          raise "timed out waiting for #{lock}; remove it only after confirming no writer is active"
        end

        Process.sleep(25)
        acquire_lock!(lock, deadline)

      {:error, reason} ->
        raise "cannot acquire #{lock}: #{inspect(reason)}"
    end
  end

  defp load_all_pointers(state) do
    # Check the yaml only where a store can exist: the root plus each
    # detected store. Never glob the whole tree.
    state
    |> store_paths()
    |> Enum.filter(&File.exists?/1)
    |> Enum.reduce(state, fn path, acc ->
      rel = Path.relative_to(path, acc.root)
      store_key = resolve_store_key(acc.submodules, rel)
      load_from_yaml(acc, store_key, path)
    end)
  end

  defp maybe_load_legacy_locked(state) do
    if not Application.get_env(:doc_pointers, :skip_legacy_import, false) and
         map_size(state.pointers) == 0 and File.exists?(legacy_json_path(state)) do
      with_lock(state, &maybe_load_legacy/1)
      |> then(fn {:reply, :ok, next} -> next end)
    else
      state
    end
  end

  defp load_from_yaml(state, store_key, path) do
    case YamlElixir.read_from_file(path) do
      {:ok, %{"pointers" => pointers}} when is_map(pointers) ->
        Enum.reduce(pointers, state, fn {uuid, data}, acc ->
          pointer = Pointer.from_map(uuid, data)
          put_pointer(acc, pointer, store_key)
        end)

      _ ->
        raise InvalidStoreError,
              "invalid pointer store at #{path}; existing YAML was left unchanged"
    end
  rescue
    error in InvalidStoreError ->
      reraise error, __STACKTRACE__

    error ->
      raise InvalidStoreError,
            "invalid pointer store at #{path}: #{Exception.message(error)}; existing YAML was left unchanged"
  end

  defp maybe_load_legacy(state) do
    legacy = legacy_json_path(state)

    if not Application.get_env(:doc_pointers, :skip_legacy_import, false) and
         map_size(state.pointers) == 0 and File.exists?(legacy) do
      state = import_legacy_json(state)
      state.store_membership |> Map.values() |> Enum.uniq() |> Enum.each(&save_store(state, &1))
      {:reply, :ok, %{state | fingerprint: store_fingerprint(state)}}
    else
      {:reply, :ok, state}
    end
  end

  defp import_legacy_json(state) do
    case File.read(legacy_json_path(state)) do
      {:ok, content} ->
        case Jason.decode(content) do
          {:ok, entries} when is_map(entries) ->
            name_counts =
              entries
              |> Enum.map(fn {token, data} -> legacy_name(token, data) end)
              |> Enum.frequencies()

            Enum.reduce(entries, state, fn {token, data}, acc ->
              name = legacy_name(token, data)

              uuid_name =
                if name_counts[name] == 1,
                  do: DocPointers.UUID5.build_name(name),
                  else: "doc-pointers:legacy-token:#{token}"

              uuid_bytes = DocPointers.UUID5.generate(uuid_name)
              uuid = DocPointers.UUID5.to_string(uuid_bytes)

              pointer =
                Pointer.new(%{
                  uuid: uuid,
                  token: token,
                  file_path: data["path"],
                  function: name,
                  description: data["description"] || "",
                  line: data["line"]
                })

              {store_key, adjusted} = resolve_and_adjust(state, pointer)
              put_pointer(acc, adjusted, store_key)
            end)

          _ ->
            state
        end

      _ ->
        state
    end
  end

  defp legacy_name(token, data) do
    case data["name"] do
      name when is_binary(name) and name != "" -> name
      _ -> token
    end
  end

  defp save_store(state, store_key) do
    dir = meta_dir(state, store_key)
    File.mkdir_p!(dir)

    store_pointers =
      state.store_membership
      |> Enum.filter(fn {_uuid, sk} -> sk == store_key end)
      |> Enum.map(fn {uuid, _} -> {uuid, state.pointers[uuid]} end)
      |> Enum.reject(fn {_, p} -> is_nil(p) end)
      |> Enum.sort_by(fn {uuid, _} -> uuid end)
      |> Enum.map(fn {uuid, pointer} -> {uuid, Pointer.to_map(pointer)} end)
      |> Map.new()

    content = Ymlr.document!(%{"pointers" => store_pointers})
    path = pointers_path(state, store_key)
    tmp = path <> ".tmp.#{System.unique_integer([:positive])}"

    try do
      File.write!(tmp, content)
      File.rename!(tmp, path)
    after
      File.rm(tmp)
    end
  end
end
