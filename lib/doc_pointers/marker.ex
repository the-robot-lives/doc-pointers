defmodule DocPointers.Marker do
  @moduledoc """
  Canonical typed UUID markers. Four-glyph tokens remain lookup aliases for
  legacy records; new source markers carry the full pointer UUID.

  Source markers always use the seven scope emoji. Stored kinds are canonical
  strings so fine-grained types (class vs struct, protocol vs behaviour) can be
  differentiated: emoji markers map to a canonical default string on store.
  """

  @string_kinds ~w(file module class struct interface protocol behaviour function logic component diagram)
  @default_kind "function"

  # Marker (emoji) space, in canonical display order.
  @kinds ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"]

  # Emoji -> canonical string kind. The value is the canonical default for that
  # emoji; sibling kinds (📦: class/struct, 🔌: protocol/behaviour) share it.
  @emoji_to_kind %{
    "📁" => "file",
    "📦" => "module",
    "🔌" => "interface",
    "🧩" => "component",
    "🔧" => "function",
    "🔀" => "logic",
    "📐" => "diagram"
  }

  @kind_to_emoji %{
    "file" => "📁",
    "module" => "📦",
    "class" => "📦",
    "struct" => "📦",
    "interface" => "🔌",
    "protocol" => "🔌",
    "behaviour" => "🔌",
    "function" => "🔧",
    "logic" => "🔀",
    "component" => "🧩",
    "diagram" => "📐"
  }

  @closable ~w(component logic diagram)
  @closable_emoji ["🧩", "🔀", "📐"]

  @uuid ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/
  @marker ~r/\A〚(\/)?(📁|📦|🔌|🧩|🔧|🔀|📐):([0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})〛\z/u
  @legacy ~r/\A⟦(.{4})⟧\z/u

  def string_kinds, do: @string_kinds
  def default_kind, do: @default_kind

  # Emoji marker space (source markers are always emoji).
  def kinds, do: @kinds
  def emoji_kinds, do: @kinds

  # Canonical string kind for an emoji or string kind; nil when unknown.
  def normalize_kind(kind) when kind in @string_kinds, do: kind
  def normalize_kind(kind) when is_binary(kind), do: @emoji_to_kind[kind]
  def normalize_kind(_), do: nil

  # Emoji for a marker; string kinds map back to their scope emoji.
  def emoji_for(kind) when is_binary(kind) do
    Map.get(@kind_to_emoji, normalize_kind(kind), kind)
  end

  def emoji_for(kind), do: kind

  # Accepts canonical strings and legacy emoji (back-compat).
  def valid_kind?(kind), do: normalize_kind(kind) != nil

  def closable?(kind) when kind in @closable_emoji, do: true
  def closable?(kind) when is_binary(kind), do: normalize_kind(kind) in @closable
  def closable?(_), do: false

  def valid_uuid?(uuid) when is_binary(uuid), do: Regex.match?(@uuid, uuid)
  def valid_uuid?(_), do: false

  def open(uuid, kind \\ @default_kind), do: "〚#{emoji_for(kind)}:#{uuid}〛"

  def close(uuid, kind) do
    if closable?(kind),
      do: "〚/#{emoji_for(kind)}:#{uuid}〛",
      else: raise(ArgumentError, "kind #{inspect(kind)} is not closable")
  end

  def declaration(uuid, kind, name, description),
    do: "#{open(uuid, kind)} #{name} :: #{description}"

  def parse(marker) when is_binary(marker) do
    case Regex.run(@marker, marker) do
      [_, closing, kind, uuid] when closing in ["", "/"] ->
        if closing == "/" and kind not in @closable_emoji do
          {:error, :not_closable}
        else
          {:ok, %{kind: kind, uuid: uuid, closing: closing == "/"}}
        end

      _ ->
        {:error, :invalid_marker}
    end
  end

  def parse_legacy(marker) when is_binary(marker) do
    case Regex.run(@legacy, marker) do
      [_, token] -> {:ok, %{token: token}}
      _ -> {:error, :invalid_legacy_marker}
    end
  end
end
