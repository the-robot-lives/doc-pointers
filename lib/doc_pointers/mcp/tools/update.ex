defmodule DocPointers.MCP.Tools.Update do
  use Noizu.MCP.Server.Tool,
    name: "doc-pointer/update",
    description:
      "Update metadata for an existing doc-pointer (description, class, line, file path).",
    annotations: [destructive_hint: true, idempotent_hint: true]

  input do
    field(:uuid, :string, description: "Full UUID of the pointer to update")
    field(:token, :string, description: "4-character token of the pointer (alternative to UUID)")
    field(:description, :string, description: "New description")
    field(:class, :string, description: "New class/module name")
    field(:line, :integer, description: "Updated line number")
    field(:file_path, :string, description: "Updated file path (if the source file moved)")

    field(:kind, :string,
      description:
        "Kind string: file, module, class, struct, interface, protocol, behaviour, " <>
          "function, logic, component, diagram. Legacy scope emoji accepted."
    )

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
    uuid =
      cond do
        args[:uuid] ->
          args.uuid

        args[:token] ->
          case DocPointers.Store.get_by_token(args.token) do
            nil -> nil
            p -> p.uuid
          end

        true ->
          nil
      end

    if is_nil(uuid) do
      {:error, "Pointer not found. Provide a valid uuid or token."}
    else
      updates = %{}

      updates =
        if args[:description], do: Map.put(updates, :description, args.description), else: updates

      updates = if args[:class], do: Map.put(updates, :class, args.class), else: updates
      updates = if args[:line], do: Map.put(updates, :line, args.line), else: updates

      updates =
        if args[:file_path], do: Map.put(updates, :file_path, args.file_path), else: updates

      updates = if args[:kind], do: Map.put(updates, :kind, args.kind), else: updates

      if args[:kind] && not DocPointers.Marker.valid_kind?(args.kind) do
        {:error,
         "kind must be one of " <>
           Enum.join(DocPointers.Marker.string_kinds(), ", ") <>
           " (legacy scope emoji accepted)"}
      else
        updates =
          if args[:kind],
            do: Map.put(updates, :kind, DocPointers.Marker.normalize_kind(args.kind)),
            else: updates

        update_pointer(uuid, updates)
      end
    end
  end

  defp update_pointer(uuid, updates) do
    case DocPointers.Store.update(uuid, updates) do
      {:ok, pointer} ->
        {:ok,
         %{
           uuid: pointer.uuid,
           token: pointer.token,
           kind: pointer.kind,
           marker: DocPointers.Marker.open(pointer.uuid, pointer.kind),
           file_path: pointer.file_path,
           class: pointer.class,
           function: pointer.function,
           line: pointer.line,
           description: pointer.description,
           updated_at: pointer.updated_at
         }}

      {:error, :not_found} ->
        {:error, "Pointer #{uuid} not found"}
    end
  end
end
