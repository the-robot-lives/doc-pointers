defmodule DocPointers.Pointer do
  alias DocPointers.Marker

  @enforce_keys [:uuid, :token, :function, :description]
  defstruct [
    :uuid,
    :token,
    :kind,
    :kind_explicit,
    :locations,
    :file_path,
    :class,
    :function,
    :line,
    :description,
    :created_at,
    :updated_at
  ]

  @type t :: %__MODULE__{
          uuid: String.t(),
          token: String.t(),
          kind: String.t(),
          kind_explicit: boolean(),
          locations: [map()],
          file_path: String.t() | nil,
          class: String.t() | nil,
          function: String.t(),
          line: non_neg_integer() | nil,
          description: String.t(),
          created_at: String.t() | nil,
          updated_at: String.t() | nil
        }

  def new(attrs) when is_map(attrs) do
    now = DateTime.utc_now() |> DateTime.to_iso8601()

    %__MODULE__{
      uuid: Map.fetch!(attrs, :uuid),
      token: Map.fetch!(attrs, :token),
      kind: Map.get(attrs, :kind, "function") |> Marker.normalize_kind() || Marker.default_kind(),
      kind_explicit: true,
      locations: Map.get(attrs, :locations, []),
      file_path: Map.get(attrs, :file_path),
      class: Map.get(attrs, :class),
      function: Map.fetch!(attrs, :function),
      line: Map.get(attrs, :line),
      description: Map.fetch!(attrs, :description),
      created_at: Map.get(attrs, :created_at, now),
      updated_at: Map.get(attrs, :updated_at, now)
    }
  end

  def to_map(%__MODULE__{} = p) do
    %{
      "token" => p.token,
      "kind" => if(p.kind_explicit, do: p.kind),
      "locations" => if(p.locations != [], do: p.locations),
      "function" => p.function,
      "file_path" => p.file_path,
      "class" => p.class,
      "line" => p.line,
      "description" => p.description,
      "created_at" => p.created_at,
      "updated_at" => p.updated_at
    }
    |> Enum.reject(fn {_k, v} -> is_nil(v) end)
    |> Map.new()
  end

  def from_map(uuid, map) when is_binary(uuid) and is_map(map) do
    %__MODULE__{
      uuid: uuid,
      token: map["token"],
      # Legacy emoji kinds normalize to canonical strings on load.
      kind: (map["kind"] && Marker.normalize_kind(map["kind"])) || "function",
      kind_explicit: Map.has_key?(map, "kind"),
      locations: map["locations"] || [],
      file_path: map["file_path"],
      class: map["class"],
      function: map["function"],
      line: map["line"],
      description: map["description"],
      created_at: map["created_at"],
      updated_at: map["updated_at"]
    }
  end
end
