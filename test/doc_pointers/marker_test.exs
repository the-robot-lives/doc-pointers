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

    assert Marker.string_kinds() == ~w(file module class struct interface protocol behaviour function logic component diagram)
    assert Marker.default_kind() == "function"
    refute Marker.valid_kind?("widget")
    assert Marker.normalize_kind("widget") == nil
  end

  test "open/close/declaration accept string kinds and render emoji markers" do
    assert Marker.open(@uuid, "function") == "〚🔧:#{@uuid}〛"
    assert Marker.open(@uuid) == "〚🔧:#{@uuid}〛"
    assert Marker.open(@uuid, "class") == "〚📦:#{@uuid}〛"
    assert Marker.declaration(@uuid, "protocol", "Auth", "does auth") ==
             "〚🔌:#{@uuid}〛 Auth :: does auth"

    assert Marker.parse(Marker.open(@uuid, "component")) ==
             {:ok, %{kind: "🧩", uuid: @uuid, closing: false}}

    assert Marker.close(@uuid, "component") == "〚/🧩:#{@uuid}〛"
    assert Marker.close(@uuid, "logic") == "〚/🔀:#{@uuid}〛"
    assert Marker.close(@uuid, "diagram") == "〚/📐:#{@uuid}〛"
    assert Marker.close(@uuid, "🧩") == "〚/🧩:#{@uuid}〛"
    assert Marker.closable?("component") and Marker.closable?("🔀")

    assert_raise ArgumentError, fn -> Marker.close(@uuid, "module") end
    assert_raise ArgumentError, fn -> Marker.close(@uuid, "function") end
  end
end
