defmodule DocPointers.MarkerTest do
  use ExUnit.Case, async: true

  alias DocPointers.Marker

  @uuid "550e8400-e29b-51d4-a716-446655440000"

  test "parses all seven typed UUID kinds" do
    for kind <- ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"] do
      assert Marker.parse(Marker.open(@uuid, kind)) ==
               {:ok, %{kind: kind, uuid: @uuid, closing: false}}
    end
  end

  test "only region kinds have closing markers" do
    for kind <- ["🧩", "🔀", "📐"] do
      assert Marker.parse(Marker.close(@uuid, kind)) ==
               {:ok, %{kind: kind, uuid: @uuid, closing: true}}
    end

    for kind <- ["📁", "📦", "🔌", "🔧"] do
      assert Marker.parse("〚/#{kind}:#{@uuid}〛") == {:error, :not_closable}
    end
  end

  test "rejects old square brackets and accepts legacy glyph lookup" do
    assert Marker.parse("[[🔀:#{@uuid}]]") == {:error, :invalid_marker}
    assert Marker.parse_legacy("⟦𓳔𔐮𔘟𔄵⟧") == {:ok, %{token: "𓳔𔐮𔘟𔄵"}}
  end
end
