defmodule DocPointers.MCP.Tools.LookupTest do
  use ExUnit.Case

  alias DocPointers.MCP.Tools.Lookup
  alias DocPointers.Store

  @uuid "550e8400-e29b-51d4-a716-446655440000"

  setup do
    root =
      Path.join(System.tmp_dir!(), "doc_pointers_lookup_#{System.unique_integer([:positive])}")

    File.mkdir_p!(Path.join(root, "lib"))
    File.write!(Path.join(root, "lib/a.ex"), "one\ntwo\nthree\nfour\n")
    File.write!(Path.join(root, "lib/b.ex"), "alpha\nbeta\ngamma\n")
    Store.set_root(root)

    record = %{
      "uuid" => @uuid,
      "kind" => "🧩",
      "file_path" => "lib/a.ex",
      "function" => "auth concern",
      "description" => "cross-cutting authentication",
      "locations" => [
        %{"file_path" => "lib/b.ex", "line" => 2, "end_line" => 3},
        %{"file_path" => "lib/a.ex", "line" => 3, "end_line" => 4}
      ]
    }

    {:ok, _} = Store.reconcile([record], true)
    on_exit(fn -> File.rm_rf!(root) end)
    {:ok, root: root}
  end

  test "plain UUID and typed selectors return every component occurrence" do
    for selector <- [@uuid, "🧩:#{@uuid}"] do
      assert {:ok, %{count: 1, results: [record]}} = Lookup.call(%{uuid: selector}, nil)
      assert record["kind"] == "🧩"
      assert Enum.map(record["snippets"], & &1["file_path"]) == ["lib/a.ex", "lib/b.ex"]
      assert Enum.at(record["snippets"], 0)["content"] =~ "three"
    end

    assert {:ok, %{count: 1}} = Lookup.call(%{marker: "〚🧩:#{@uuid}〛"}, nil)
    assert {:ok, %{count: 0}} = Lookup.call(%{uuid: "🔀:#{@uuid}"}, nil)
  end

  test "missing source is reported without failing lookup", %{root: root} do
    File.rm!(Path.join(root, "lib/b.ex"))
    assert {:ok, %{results: [record]}} = Lookup.call(%{uuid: @uuid}, nil)
    assert Enum.at(record["snippets"], 1)["error"] =~ "source unavailable"
  end
end
