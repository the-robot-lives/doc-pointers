defmodule DocPointers.MCP.Tools.Generate do
  use Noizu.MCP.Server.Tool,
    name: "doc-pointer/generate",
    description: """
    Generate a typed pointer marker for a source location and write its metadata
    to .meta/pointers.yaml. Markers embed the pointer's 4-glyph token; the
    full UUID is returned alongside and remains accepted in lookups.
    """,
    annotations: [destructive_hint: true]

  input do
    field(:file_path, :string,
      required: true,
      description: "Relative path to the source file (e.g. lib/my_module.ex)"
    )

    field(:function_name, :string,
      required: true,
      description: "Function or method name"
    )

    field(:description, :string,
      required: true,
      description: "Human-readable description of what this code location does"
    )

    field(:kind, :string,
      description:
        "Kind string: file, module, class, struct, interface, protocol, behaviour, " <>
          "function (default), logic, component, diagram. Legacy scope emoji " <>
          "(📁 📦 🔌 🧩 🔧 🔀 📐) are accepted and normalized."
    )

    field(:class, :string, description: "Module or class name (e.g. MyApp.Auth)")
    field(:line, :integer, description: "Line number in the source file")
    field(:salt, :string, description: "Optional deterministic salt for UUID derivation")

    field(:name_override, :string,
      description: "Override the auto-derived name (default: file_path::function_name)"
    )

    field(:confirm, :boolean,
      description: "Required true unless the server was started with --write"
    )
  end

  @max_attempts 10_000

  @impl true
  def call(args, ctx) do
    with :ok <- DocPointers.MCP.Writes.authorize(args, ctx) do
      register(%{
        base_name:
          args[:name_override] ||
            DocPointers.UUID5.build_annotation_name(args.file_path, args.function_name),
        file_path: args.file_path,
        kind: args[:kind],
        class: args[:class],
        function: args.function_name,
        line: args[:line],
        description: args.description,
        salt: args[:salt]
      })
    end
  end

  @doc """
  Shared registration pipeline: mint a UUIDv5 + hieroglyph token for `base_name`,
  persist the pointer, and return its metadata map. Used by doc-pointer/generate
  and doc-pointer/generate-batch.

  Attrs: base_name (required), function (required), file_path, kind, class, line,
  description, salt.
  """
  def register(%{base_name: base_name, function: function} = attrs) do
    with :ok <- validate_kind(attrs[:kind]) do
      kind = (attrs[:kind] && DocPointers.Marker.normalize_kind(attrs.kind)) || "function"

      case generate_with_collision_check(base_name, attrs[:salt], 0) do
        {:ok, uuid_string, token} ->
          pointer =
            DocPointers.Pointer.new(%{
              uuid: uuid_string,
              token: token,
              kind: kind,
              file_path: attrs[:file_path],
              class: attrs[:class],
              function: function,
              line: attrs[:line],
              description: attrs[:description] || ""
            })

          DocPointers.Store.put(pointer)

          {:ok,
           %{
             uuid: uuid_string,
             token: token,
             kind: pointer.kind,
             marker: DocPointers.Marker.open(uuid_string, pointer.kind),
             close_marker: maybe_close(uuid_string, pointer.kind),
             declaration:
               DocPointers.Marker.declaration(
                 uuid_string,
                 pointer.kind,
                 function,
                 attrs[:description] || ""
               ),
             file_path: attrs[:file_path],
             function: function,
             class: attrs[:class],
             description: attrs[:description] || ""
           }}

        {:error, :max_attempts} ->
          {:error, "Failed to generate a unique token after #{@max_attempts} attempts"}
      end
    end
  end

  defp validate_kind(nil), do: :ok

  defp validate_kind(kind) do
    if DocPointers.Marker.valid_kind?(kind),
      do: :ok,
      else:
        {:error,
         "kind must be one of " <>
           Enum.join(DocPointers.Marker.string_kinds(), ", ") <>
           " (legacy scope emoji accepted)"}
  end

  defp maybe_close(uuid, kind) do
    if DocPointers.Marker.closable?(kind), do: DocPointers.Marker.close(uuid, kind)
  end

  defp generate_with_collision_check(_base_name, _salt, attempt) when attempt >= @max_attempts do
    {:error, :max_attempts}
  end

  defp generate_with_collision_check(base_name, salt, attempt) do
    name = DocPointers.UUID5.build_name(base_name, salt, attempt)
    uuid_bytes = DocPointers.UUID5.generate(name)
    uuid_string = DocPointers.UUID5.to_string(uuid_bytes)
    token = DocPointers.Hieroglyph.encode(uuid_bytes)

    if DocPointers.Store.token_exists?(token) do
      generate_with_collision_check(base_name, salt, attempt + 1)
    else
      {:ok, uuid_string, token}
    end
  end
end
