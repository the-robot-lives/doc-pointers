defmodule DocPointers.Marker do
  @moduledoc """
  Canonical typed markers. New markers embed the pointer's 4-glyph hieroglyph
  token — `〚{emoji}:{token}〛` — derived from the UUIDv5 via the existing
  encode path (tokens are never invented). Full UUIDv5 payloads remain
  accepted everywhere markers are parsed or looked up; token payloads
  resolve to UUIDs through the store's token index.

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

  @uuid_body ~S/[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/
  @token_ranges ~S/[\x{10980}-\x{1099F}\x{13000}-\x{1342F}\x{13460}-\x{143FF}\x{14400}-\x{1467F}]/
  @uuid ~r/\A#{@uuid_body}\z/
  @token ~r/\A#{@token_ranges}{4}\z/u
  # payload: full UUIDv5 (backward compatible) or exactly 4 hieroglyph-range glyphs
  @marker ~r/\A〚(\/)?(📁|📦|🔌|🧩|🔧|🔀|📐):(#{@uuid_body}|#{@token_ranges}{4})〛\z/u
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
  def valid_token?(token) when is_binary(token), do: Regex.match?(@token, token)
  def valid_token?(_), do: false

  @doc """
  Derive the 4-glyph token for a v5 UUID string (existing encode path only).
  """
  def token_from_uuid(uuid) when is_binary(uuid) do
    if valid_uuid?(uuid) do
      uuid |> DocPointers.UUID5.from_string() |> DocPointers.Hieroglyph.encode()
    end
  end

  def token_from_uuid(_), do: nil

  @doc """
  Open marker `〚{emoji}:{token}〛`; the token is derived from the UUID.
  """
  def open(uuid, kind \\ @default_kind) do
    token = token_from_uuid(uuid) || uuid
    "〚#{emoji_for(kind)}:#{token}〛"
  end

  def close(uuid, kind) do
    if closable?(kind) do
      token = token_from_uuid(uuid) || uuid
      "〚/#{emoji_for(kind)}:#{token}〛"
    else
      raise(ArgumentError, "kind #{inspect(kind)} is not closable")
    end
  end

  def declaration(uuid, kind, name, description),
    do: "#{open(uuid, kind)} #{name} :: #{description}"

  @doc """
  Parse a canonical marker. Returns `{:ok, %{kind:, closing:, uuid:}}` for a
  full-UUID payload or `{:ok, %{kind:, closing:, token:}}` for a 4-glyph
  token payload; the caller resolves tokens to UUIDs via the store.
  """
  def parse(marker) when is_binary(marker) do
    case Regex.run(@marker, marker) do
      [_, closing, kind, payload] when closing in ["", "/"] ->
        if closing == "/" and kind not in @closable_emoji do
          {:error, :not_closable}
        else
          payload_key = if valid_uuid?(payload), do: :uuid, else: :token

          {:ok, %{kind: kind, closing: closing == "/"} |> Map.put(payload_key, payload)}
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
