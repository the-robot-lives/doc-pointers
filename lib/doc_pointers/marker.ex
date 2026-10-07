defmodule DocPointers.Marker do
  @moduledoc """
  Canonical typed markers. New markers embed the pointer's 4-glyph hieroglyph
  token — `〚{emoji}:{token}〛` — derived from the UUIDv5 via the existing
  encode path (tokens are never invented). Full UUIDv5 payloads remain
  accepted everywhere markers are parsed; token payloads resolve to UUIDs
  through the store's token index.
  """

  @kinds ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"]
  @closable ["🧩", "🔀", "📐"]
  @uuid_body ~S/[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/
  @uuid Regex.compile!("\\A#{@uuid_body}\\z")
  @token_ranges ~S/[\x{10980}-\x{1099F}\x{13000}-\x{1342F}\x{13460}-\x{143FF}\x{14400}-\x{1467F}]/
  @token Regex.compile!("\\A#{@token_ranges}{4}\\z", "u")
  @marker Regex.compile!("\\A〚(\\/)?(📁|📦|🔌|🧩|🔧|🔀|📐):(#{@uuid_body}|#{@token_ranges}{4})〛\\z", "u")

  def kinds, do: @kinds
  def valid_kind?(kind), do: kind in @kinds
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
  Open marker `〚{kind}:{token}〛`; the token is derived from the UUID.
  """
  def open(uuid, kind \\ "🔧") do
    token = token_from_uuid(uuid) || uuid
    "〚#{kind}:#{token}〛"
  end

  def close(uuid, kind) when kind in @closable do
    token = token_from_uuid(uuid) || uuid
    "〚/#{kind}:#{token}〛"
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
        if closing == "/" and kind not in @closable do
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
    case Regex.run(~r/\A⟦(.{4})⟧\z/u, marker) do
      [_, token] -> {:ok, %{token: token}}
      _ -> {:error, :invalid_legacy_marker}
    end
  end
end
