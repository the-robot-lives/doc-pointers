defmodule DocPointers.StoreReconcileTest do
  use ExUnit.Case

  alias DocPointers.Store

  setup do
    root =
      Path.join(System.tmp_dir!(), "doc_pointers_reconcile_#{System.unique_integer([:positive])}")

    File.mkdir_p!(root)
    Store.set_root(root)
    on_exit(fn -> File.rm_rf!(root) end)
    {:ok, root: root}
  end

  test "dry run is read-only and write preserves identity and unrelated records", %{root: root} do
    first = record("𓳔𔐮𔘟𔄵", "lib/a.ex", "a")
    second = record("𓀀𓀻𓃉𓏦", "lib/b.ex", "b")

    assert {:ok, %{changed: true, inserted: 1}} = Store.reconcile([first], false)
    refute File.exists?(Path.join(root, ".meta"))
    assert Store.snapshot().records == []

    assert {:ok, %{inserted: 2}} = Store.reconcile([first, second], true)
    records = Store.snapshot().records
    old = Enum.find(records, &(&1["token"] == first["token"]))

    moved = %{first | "file_path" => "lib/moved.ex", "description" => "updated"}
    assert {:ok, %{changed: true, updated: 1, inserted: 0}} = Store.reconcile([moved], true)
    records = Store.snapshot().records
    current = Enum.find(records, &(&1["token"] == first["token"]))

    assert current["uuid"] == old["uuid"]
    assert current["created_at"] == old["created_at"]
    assert current["file_path"] == "lib/moved.ex"
    assert length(records) == 2
    assert {:ok, %{changed: false}} = Store.reconcile([moved], true)
  end

  test "reports identity conflicts without changing YAML", %{root: root} do
    first = record("𓳔𔐮𔘟𔄵", "lib/a.ex", "a")
    {:ok, _} = Store.reconcile([first], true)
    path = Path.join([root, ".meta", "pointers.yaml"])
    before = File.read!(path)

    assert {:error, message} =
             Store.reconcile([Map.put(first, "uuid", "different-uuid")], true)

    assert message =~ "different UUID"
    assert File.read!(path) == before
  end

  test "running Store refreshes external YAML changes before reads and writes", %{root: root} do
    first = record("𓳔𔐮𔘟𔄵", "lib/a.ex", "a")
    {:ok, _} = Store.reconcile([first], true)
    path = Path.join([root, ".meta", "pointers.yaml"])
    {:ok, data} = YamlElixir.read_from_file(path)

    external = %{
      "token" => "𓀀𓀻𓃉𓏦",
      "file_path" => "lib/external.ex",
      "function" => "external",
      "description" => "external record"
    }

    data = put_in(data, ["pointers", "external-uuid"], external)
    File.write!(path <> ".new", Ymlr.document!(data))
    File.rename!(path <> ".new", path)

    assert {:ok, on_disk} = YamlElixir.read_from_file(path)
    assert on_disk["pointers"]["external-uuid"] == external
    assert Store.get("external-uuid").token == external["token"]
    {:ok, _} = Store.reconcile([record("𓃉𓏦𓀀𓀻", "lib/c.ex", "c")], true)
    {:ok, persisted} = YamlElixir.read_from_file(path)
    assert persisted["pointers"]["external-uuid"] == external
  end

  defp record(token, file_path, function) do
    %{
      "token" => token,
      "file_path" => file_path,
      "function" => function,
      "description" => "description for #{function}"
    }
  end
end
