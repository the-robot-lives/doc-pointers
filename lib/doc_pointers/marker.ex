defmodule DocPointers.Marker do
  @moduledoc """
  Canonical typed UUID markers. Four-glyph tokens remain lookup aliases for
  legacy records; new source markers carry the full pointer UUID.
  """

  @kinds ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"]
  @closable ["🧩", "🔀", "📐"]
  @uuid ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/
  @marker ~r/\A〚(\/)?(📁|📦|🔌|🧩|🔧|🔀|📐):([0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})〛\z/u
  @legacy ~r/\A⟦(.{4})⟧\z/u

  def kinds, do: @kinds
  def valid_kind?(kind), do: kind in @kinds
  def valid_uuid?(uuid) when is_binary(uuid), do: Regex.match?(@uuid, uuid)
  def valid_uuid?(_), do: false

  def open(uuid, kind \\ "🔧"), do: "〚#{kind}:#{uuid}〛"

  def close(uuid, kind) when kind in @closable, do: "〚/#{kind}:#{uuid}〛"

  def declaration(uuid, kind, name, description),
    do: "#{open(uuid, kind)} #{name} :: #{description}"

  def parse(marker) when is_binary(marker) do
    case Regex.run(@marker, marker) do
      [_, closing, kind, uuid] when closing in ["", "/"] ->
        if closing == "/" and kind not in @closable do
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
