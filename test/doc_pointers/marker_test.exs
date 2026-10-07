defmodule DocPointers.MarkerTest do
  use ExUnit.Case, async: true

  alias DocPointers.Marker

  @uuid "550e8400-e29b-51d4-a716-446655440000"
  @token Marker.token_from_uuid(@uuid)
  # golden fixture: uuid5("doc-pointers:TestPointer") -> 𓳔𔐮𔘟𔄵
  @golden_uuid "5c692577-ad0c-51f1-992c-759b5e5fffb5"
  @golden_token "𓳔𔐮𔘟𔄵"

  test "open/close embed the derived 4-glyph token" do
    assert Marker.open(@golden_uuid) == "〚🔧:#{@golden_token}〛"
    assert Marker.close(@golden_uuid, "🧩") == "〚/🧩:#{@golden_token}〛"
    assert Marker.token_from_uuid(@golden_uuid) == @golden_token
  end

  test "parses token-form markers and resolves via caller" do
    assert Marker.parse("〚🔧:#{@golden_token}〛") ==
             {:ok, %{kind: "🔧", token: @golden_token, closing: false}}

    assert Marker.parse("〚/📐:#{@golden_token}〛") ==
             {:ok, %{kind: "📐", token: @golden_token, closing: true}}
  end

  test "full UUID payloads remain accepted" do
    for kind <- ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"] do
      assert Marker.parse("〚#{kind}:#{@uuid}〛") ==
               {:ok, %{kind: kind, uuid: @uuid, closing: false}}
    end
  end

  test "only region kinds have closing markers" do
    for kind <- ["🧩", "🔀", "📐"] do
      assert Marker.parse("〚/#{kind}:#{@token}〛") ==
               {:ok, %{kind: kind, token: @token, closing: true}}
    end

    for kind <- ["📁", "📦", "🔌", "🔧"] do
      assert Marker.parse("〚/#{kind}:#{@uuid}〛") == {:error, :not_closable}
      assert Marker.parse("〚/#{kind}:#{@token}〛") == {:error, :not_closable}
    end
  end

  test "rejects malformed tokens and old square brackets" do
    # wrong glyph count, non-hieroglyph characters
    assert Marker.parse("〚🔧:𓳔𔐮𔘟〛") == {:error, :invalid_marker}
    assert Marker.parse("〚🔧:abCD〛") == {:error, :invalid_marker}
    assert Marker.parse("〚🔧:𓳔𔐮𔘟𔄵𓳔〛") == {:error, :invalid_marker}
    assert Marker.parse("[[🔀:#{@uuid}]]") == {:error, :invalid_marker}
    assert Marker.parse_legacy("⟦𓳔𔐮𔘟𔄵⟧") == {:ok, %{token: "𓳔𔐮𔘟𔄵"}}
  end

  test "valid_token? guards the hieroglyph ranges" do
    assert Marker.valid_token?(@golden_token)
    refute Marker.valid_token?("𓳔𔐮𔘟")
    refute Marker.valid_token?("abCD")
    refute Marker.valid_token?(@uuid)
  end
end
