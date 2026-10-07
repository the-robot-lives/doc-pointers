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

  test "normalizes emoji and string kinds to the canonical string space" do
    emoji_defaults = %{
      "📁" => "file",
      "📦" => "module",
      "🔌" => "interface",
      "🧩" => "component",
      "🔧" => "function",
      "🔀" => "logic",
      "📐" => "diagram"
    }

    for {emoji, string} <- emoji_defaults do
      assert Marker.normalize_kind(emoji) == string
      assert Marker.valid_kind?(emoji)
      assert Marker.emoji_for(string) == emoji
    end

    # Fine-grained siblings of 📦 and 🔌 keep their own string kind but share
    # the emoji scope in markers.
    for fine <- ["class", "struct"] do
      assert Marker.normalize_kind(fine) == fine
      assert Marker.emoji_for(fine) == "📦"
    end

    for fine <- ["protocol", "behaviour"] do
      assert Marker.normalize_kind(fine) == fine
      assert Marker.emoji_for(fine) == "🔌"
    end

    assert Marker.string_kinds() ==
             ~w(file module class struct interface protocol behaviour function logic component diagram)

    assert Marker.default_kind() == "function"
    refute Marker.valid_kind?("widget")
    assert Marker.normalize_kind("widget") == nil
  end

  test "open/close/declaration accept string kinds and render emoji markers with token payloads" do
    assert Marker.open(@uuid, "function") == "〚🔧:#{@token}〛"
    assert Marker.open(@uuid) == "〚🔧:#{@token}〛"
    assert Marker.open(@uuid, "class") == "〚📦:#{@token}〛"

    assert Marker.declaration(@uuid, "protocol", "Auth", "does auth") ==
             "〚🔌:#{@token}〛 Auth :: does auth"

    assert Marker.parse(Marker.open(@uuid, "component")) ==
             {:ok, %{kind: "🧩", token: @token, closing: false}}

    assert Marker.close(@uuid, "component") == "〚/🧩:#{@token}〛"
    assert Marker.close(@uuid, "logic") == "〚/🔀:#{@token}〛"
    assert Marker.close(@uuid, "diagram") == "〚/📐:#{@token}〛"
    assert Marker.close(@uuid, "🧩") == "〚/🧩:#{@token}〛"
    assert Marker.closable?("component") and Marker.closable?("🔀")

    assert_raise ArgumentError, fn -> Marker.close(@uuid, "module") end
    assert_raise ArgumentError, fn -> Marker.close(@uuid, "function") end
  end
end
