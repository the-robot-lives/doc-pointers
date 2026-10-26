defmodule DocPointers.MCP.Tools.GenerateBatchTest do
  use ExUnit.Case

  alias DocPointers.MCP.Tools.GenerateBatch
  alias DocPointers.Store

  setup do
    tmp_dir = System.tmp_dir!() |> Path.join("doc_pointers_batch_test_#{:rand.uniform(100_000)}")
    File.mkdir_p!(tmp_dir)
    Store.set_root(tmp_dir)
    Application.put_env(:doc_pointers, :mcp_writes, true)

    on_exit(fn ->
      Application.delete_env(:doc_pointers, :mcp_writes)
      File.rm_rf!(tmp_dir)
    end)

    {:ok, root: tmp_dir}
  end

  defp touch!(path) do
    path |> Path.dirname() |> File.mkdir_p!()
    File.touch!(path)
    path
  end

  describe "call/2" do
    test "registers multiple entries and returns name/uuid pairs" do
      touch!(Path.join(Store.root(), "lib/my_app/auth.ex"))
      touch!(Path.join(Store.root(), "lib/my_app/repo.ex"))

      args = %{
        entries: [
          %{
            name: "MyMethod",
            kind: "function",
            location: "lib/my_app/auth.ex",
            description: "A test method for exercising doc-pointer meta data"
          },
          %{
            name: "MyRepo",
            kind: "module",
            class: "MyApp.Repo",
            line: 42,
            location: "lib/my_app/repo.ex",
            description: "The repo module"
          }
        ]
      }

      {:ok, result} = GenerateBatch.call(args, nil)

      assert result.count == 2
      assert result.total == 2
      assert result.failed == []

      [first, second] = result.registered
      assert %{name: "MyMethod", uuid: uuid1, token: token1, marker: marker1} = first
      assert %{name: "MyRepo", uuid: uuid2} = second

      assert String.length(uuid1) == 36
      assert String.length(token1) == 4
      assert marker1 == "〚🔧:#{token1}〛"
      assert first.status == :ok
      assert first.kind == "function"

      # Persisted and retrievable
      stored = Store.get(uuid1)
      assert stored.function == "MyMethod"
      assert stored.kind == "function"
      assert stored.description == "A test method for exercising doc-pointer meta data"
      second_stored = Store.get(uuid2)
      assert second_stored.function == "MyRepo"
      assert second_stored.kind == "module"
      assert second_stored.class == "MyApp.Repo"
      assert second_stored.line == 42
    end

    test "same name in different files derives distinct pointers" do
      touch!(Path.join(Store.root(), "lib/a/auth.ex"))
      touch!(Path.join(Store.root(), "lib/b/auth.ex"))

      args = %{
        entries: [
          %{name: "login", location: "lib/a/auth.ex", description: "authenticates in module a"},
          %{name: "login", location: "lib/b/auth.ex", description: "authenticates in module b"}
        ]
      }

      {:ok, result} = GenerateBatch.call(args, nil)

      assert result.failed == []
      [%{uuid: uuid1}, %{uuid: uuid2}] = result.registered
      refute uuid1 == uuid2
      assert Store.get(uuid1).file_path == "lib/a/auth.ex"
      assert Store.get(uuid2).file_path == "lib/b/auth.ex"
    end

    test "absolute paths are verified and relativized into submodule metadata" do
      root = Store.root()
      sub = Path.join(root, "mysub")
      File.mkdir_p!(Path.join(sub, ".git"))
      # Store snapshots submodules at root-config time — rescan like a fresh server start
      Store.set_root(root)
      abs_path = touch!(Path.join(sub, "lib/inner.ex"))

      args = %{
        entries: [
          %{
            name: "InnerFn",
            kind: "function",
            location: abs_path,
            description: "Lives inside a submodule"
          }
        ]
      }

      {:ok, result} = GenerateBatch.call(args, nil)

      assert result.failed == []
      [entry] = result.registered
      # Response location is project-root-relative...
      assert entry.location == "mysub/lib/inner.ex"

      # ...while the store strips the submodule prefix and files the pointer
      # into the submodule's own .meta/pointers.yaml
      stored = Store.get(entry.uuid)
      assert stored.file_path == "lib/inner.ex"
      assert File.exists?(Path.join([root, "mysub", ".meta", "pointers.yaml"]))
      refute File.exists?(Path.join([root, ".meta", "pointers.yaml"]))
    end

    test "missing file fails that entry only" do
      touch!(Path.join(Store.root(), "lib/exists.ex"))

      args = %{
        entries: [
          %{name: "Good", kind: "function", location: "lib/exists.ex", description: "ok"},
          %{name: "Bad", kind: "function", location: "lib/nope.ex", description: "missing"}
        ]
      }

      {:ok, result} = GenerateBatch.call(args, nil)

      assert result.count == 1
      assert result.total == 2
      assert [%{name: "Good"}] = result.registered

      assert [%{name: "Bad", status: :error, error: error}] = result.failed
      assert error =~ "file not found"
    end

    test "entry missing name fails validation" do
      touch!(Path.join(Store.root(), "lib/x.ex"))

      args = %{
        entries: [%{kind: "function", location: "lib/x.ex", description: "no name"}]
      }

      {:ok, result} = GenerateBatch.call(args, nil)
      assert result.count == 0
      assert [%{status: :error, error: error}] = result.failed
      assert error =~ "missing required field: name"
    end

    test "invalid kind fails that entry" do
      touch!(Path.join(Store.root(), "lib/kind.ex"))

      args = %{
        entries: [%{name: "K", location: "lib/kind.ex", kind: "bogus", description: "bad kind"}]
      }

      {:ok, result} = GenerateBatch.call(args, nil)
      assert result.count == 0
      assert [%{status: :error, error: error}] = result.failed
      assert error =~ "kind must be one of"
    end

    test "absolute path outside project root is rejected" do
      outside = System.tmp_dir!() |> Path.join("doc_pointers_outside_#{:rand.uniform(100_000)}")
      touch!(outside)

      on_exit(fn -> File.rm_rf!(outside) end)

      args = %{
        entries: [%{name: "Far", location: outside, description: "not under root"}]
      }

      {:ok, result} = GenerateBatch.call(args, nil)
      assert result.count == 0
      assert [%{error: error}] = result.failed
      assert error =~ "outside the project root"
    end

    test "rejects empty or missing entries" do
      {:error, msg} = GenerateBatch.call(%{entries: []}, nil)
      assert msg =~ "non-empty array"

      {:error, msg} = GenerateBatch.call(%{}, nil)
      assert msg =~ "non-empty array"
    end

    test "requires write access when writes are disabled" do
      Application.put_env(:doc_pointers, :mcp_writes, false)

      {:error, msg} =
        GenerateBatch.call(
          %{entries: [%{name: "X", location: "lib/x.ex", description: "denied"}]},
          nil
        )

      assert msg =~ "Write tools are disabled"
    after
      Application.put_env(:doc_pointers, :mcp_writes, true)
    end
  end
end
