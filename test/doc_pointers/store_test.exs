defmodule DocPointers.StoreTest do
  use ExUnit.Case

  alias DocPointers.{Pointer, Store}

  setup do
    tmp_dir = System.tmp_dir!() |> Path.join("doc_pointers_test_#{:rand.uniform(100_000)}")
    File.mkdir_p!(tmp_dir)
    Store.set_root(tmp_dir)

    on_exit(fn -> File.rm_rf!(tmp_dir) end)

    {:ok, root: tmp_dir}
  end

  describe "put and get" do
    test "stores and retrieves a pointer by UUID" do
      pointer = make_pointer("test-uuid-1", "𓳔𔐮𔘟𔄵")
      Store.put(pointer)

      result = Store.get("test-uuid-1")
      assert result.uuid == "test-uuid-1"
      assert result.token == "𓳔𔐮𔘟𔄵"
      assert result.function == "login"
    end

    test "retrieves by token" do
      pointer = make_pointer("test-uuid-2", "𓀀𓀻𓃉𓏦")
      Store.put(pointer)

      result = Store.get_by_token("𓀀𓀻𓃉𓏦")
      assert result.uuid == "test-uuid-2"
    end

    test "returns nil for unknown UUID" do
      assert Store.get("nonexistent") == nil
    end

    test "returns nil for unknown token" do
      assert Store.get_by_token("𓀀𓀀𓀀𓀀") == nil
    end
  end

  describe "token_exists?" do
    test "returns true for existing token" do
      pointer = make_pointer("uuid-exists", "𓃉𓏦𓀀𓀻")
      Store.put(pointer)

      assert Store.token_exists?("𓃉𓏦𓀀𓀻")
    end

    test "returns false for unknown token" do
      refute Store.token_exists?("𓀀𓀀𓀀𓀀")
    end
  end

  describe "update" do
    test "updates description" do
      pointer = make_pointer("uuid-update", "𓏦𓀀𓀻𓃉")
      Store.put(pointer)

      {:ok, updated} = Store.update("uuid-update", %{description: "new desc"})
      assert updated.description == "new desc"
      assert updated.function == "login"

      reloaded = Store.get("uuid-update")
      assert reloaded.description == "new desc"
    end

    test "returns error for nonexistent UUID" do
      assert Store.update("nope", %{description: "x"}) == {:error, :not_found}
    end
  end

  describe "list" do
    test "returns all pointers with total count" do
      Store.put(make_pointer("a", "𓀀𓀻𓃉𓏦", file_path: "lib/a.ex"))
      Store.put(make_pointer("b", "𓳔𔐮𔘟𔄵", file_path: "lib/b.ex"))

      {pointers, total} = Store.list()
      assert total == 2
      assert length(pointers) == 2
    end

    test "filters by file_prefix" do
      Store.put(make_pointer("c", "𓀀𓃉𓏦𓀻", file_path: "lib/auth/login.ex"))
      Store.put(make_pointer("d", "𓳔𔘟𔐮𔄵", file_path: "test/auth_test.exs"))

      {pointers, total} = Store.list(file_prefix: "lib/")
      assert total == 1
      assert hd(pointers).file_path == "lib/auth/login.ex"
    end

    test "filters by class" do
      Store.put(make_pointer("e", "𓀻𓃉𓏦𓀀", class: "MyApp.Auth"))
      Store.put(make_pointer("f", "𔐮𓳔𔘟𔄵", class: "MyApp.Repo"))

      {pointers, _} = Store.list(class: "MyApp.Auth")
      assert length(pointers) == 1
      assert hd(pointers).class == "MyApp.Auth"
    end

    test "pagination" do
      for i <- 1..5 do
        token_char = <<0x13000 + i::utf8>>
        token = String.duplicate(token_char, 4)
        Store.put(make_pointer("page-#{i}", token))
      end

      {pointers, total} = Store.list(limit: 2, offset: 0)
      assert total == 5
      assert length(pointers) == 2
    end
  end

  describe "persistence" do
    test "writes .meta/pointers.yaml", %{root: root} do
      Store.put(make_pointer("persist-1", "𓳔𔐮𔘟𔄵"))

      yaml_path = Path.join([root, ".meta", "pointers.yaml"])
      assert File.exists?(yaml_path)

      {:ok, data} = YamlElixir.read_from_file(yaml_path)
      assert data["pointers"]["persist-1"]["token"] == "𓳔𔐮𔘟𔄵"
    end
  end

  describe "legacy import" do
    test "imports from docs/doc-pointer-db.json", %{root: root} do
      docs_dir = Path.join(root, "docs")
      File.mkdir_p!(docs_dir)

      json =
        Jason.encode!(%{
          "𓀀𓀻𓃉𓏦" => %{
            "path" => "lib/auth.ex",
            "line" => 42,
            "name" => "login",
            "description" => "auto-generated pointer"
          }
        })

      File.write!(Path.join(docs_dir, "doc-pointer-db.json"), json)

      Store.set_root(root)

      all = Store.all()
      assert length(all) >= 1
      imported = Enum.find(all, fn p -> p.token == "𓀀𓀻𓃉𓏦" end)
      assert imported != nil
      assert imported.file_path == "lib/auth.ex"
      assert imported.function == "login"
    end

    test "keeps every record when legacy names repeat", %{root: root} do
      docs_dir = Path.join(root, "docs")
      File.mkdir_p!(docs_dir)

      entries = %{
        "𓀀𓀻𓃉𓏦" => %{"path" => "lib/a.ex", "name" => "login", "description" => "first"},
        "𓳔𔐮𔘟𔄵" => %{"path" => "lib/b.ex", "name" => "login", "description" => "second"},
        "𓃉𓏦𓀀𓀻" => %{"path" => "lib/c.ex", "name" => "unique", "description" => "third"}
      }

      File.write!(Path.join(docs_dir, "doc-pointer-db.json"), Jason.encode!(entries))
      Store.set_root(root)

      assert length(Store.all()) == 3
      records = Store.snapshot().records

      for token <- ["𓀀𓀻𓃉𓏦", "𓳔𔐮𔘟𔄵"] do
        expected =
          "doc-pointers:legacy-token:#{token}"
          |> DocPointers.UUID5.generate()
          |> DocPointers.UUID5.to_string()

        assert Enum.any?(records, &(&1["token"] == token and &1["uuid"] == expected))
      end

      unique_uuid =
        "unique"
        |> DocPointers.UUID5.build_name()
        |> DocPointers.UUID5.generate()
        |> DocPointers.UUID5.to_string()

      assert Enum.any?(records, &(&1["token"] == "𓃉𓏦𓀀𓀻" and &1["uuid"] == unique_uuid))

      {:ok, yaml} = YamlElixir.read_from_file(Path.join([root, ".meta", "pointers.yaml"]))
      assert map_size(yaml["pointers"]) == 3
    end
  end

  describe "nested submodule stores" do
    test "loads a nested .meta/pointers.yaml from its own folder", %{root: root} do
      sub = Path.join([root, "Portfolio", "Libs", "ai", "genai"])
      File.mkdir_p!(Path.join(sub, ".meta"))

      {:ok, yaml} =
        Ymlr.document(%{
          "pointers" => %{
            "nested-1" => %{
              "token" => "𓀀𓀻𓃉𓏦",
              "file_path" => "lib/nested.ex",
              "function" => "nested_fn",
              "description" => "nested pointer"
            }
          }
        })

      File.write!(Path.join([sub, ".meta", "pointers.yaml"]), yaml)

      # nested checkout acts as its own store even without a .gitmodules entry
      File.mkdir_p!(Path.join(sub, ".git"))
      Store.set_root(root)

      nested = Enum.find(Store.all(), fn p -> p.uuid == "nested-1" end)
      assert nested.file_path == "lib/nested.ex"
    end

    test "ignores git repos and yamls inside deps/node_modules/_build", %{root: root} do
      vendored = Path.join([root, "deps", "vendored"])
      File.mkdir_p!(Path.join(vendored, ".git"))

      node_mod = Path.join([root, "sub", "node_modules", "pkg"])
      File.mkdir_p!(Path.join(node_mod, ".git"))

      Store.set_root(root)

      # a vendored repo must not become a store: pointer routes to root yaml
      Store.put(make_pointer("vendored-1", "𓀀𓀻𓃉𓏦", file_path: "deps/vendored/lib/x.ex"))
      {:ok, data} = YamlElixir.read_from_file(Path.join([root, ".meta", "pointers.yaml"]))
      assert Map.has_key?(data["pointers"], "vendored-1")
      assert Store.get("vendored-1").file_path == "deps/vendored/lib/x.ex"
      refute File.exists?(Path.join([vendored, ".meta", "pointers.yaml"]))

      Store.put(make_pointer("nm-1", "𓳔𔐮𔘟𔄵", file_path: "sub/node_modules/pkg/lib/y.ex"))
      {:ok, data} = YamlElixir.read_from_file(Path.join([root, ".meta", "pointers.yaml"]))
      assert Map.has_key?(data["pointers"], "nm-1")
      refute File.exists?(Path.join([node_mod, ".meta", "pointers.yaml"]))
    end

    test "saves new submodule pointer into submodule store", %{root: root} do
      sub = Path.join(root, "sub")
      File.mkdir_p!(Path.join(sub, ".git"))
      Store.set_root(root)

      Store.put(make_pointer("sub-1", "𓀀𓀻𓃉𓏦", file_path: "sub/lib/x.ex"))

      assert File.exists?(Path.join([sub, ".meta", "pointers.yaml"]))
      refute File.exists?(Path.join([root, ".meta", "pointers.yaml"]))
    end

    test "update relocates pointer across store boundary", %{root: root} do
      sub = Path.join(root, "sub")
      File.mkdir_p!(Path.join(sub, ".git"))
      Store.set_root(root)

      Store.put(make_pointer("mover", "𓀀𓀻𓃉𓏦"))

      {:ok, _} = Store.update("mover", %{file_path: "sub/lib/moved.ex"})

      assert Store.get("mover").file_path == "lib/moved.ex"
      assert File.exists?(Path.join([sub, ".meta", "pointers.yaml"]))
      {:ok, data} = YamlElixir.read_from_file(Path.join([root, ".meta", "pointers.yaml"]))
      assert data["pointers"] == %{}
    end
  end

  describe "git superproject detection" do
    test "routes pointers into a nested registered submodule", %{root: root} do
      git = fn args, dir ->
        {out, 0} =
          System.cmd("git", ["-c", "protocol.file.allow=always" | args],
            cd: dir,
            stderr_to_stdout: true,
            env: [
              {"GIT_AUTHOR_NAME", "t"},
              {"GIT_AUTHOR_EMAIL", "t@t"},
              {"GIT_COMMITTER_NAME", "t"},
              {"GIT_COMMITTER_EMAIL", "t@t"}
            ]
          )

        out
      end

      origin = Path.join(Path.dirname(root), Path.basename(root) <> "_origin")
      File.mkdir_p!(origin)
      on_exit(fn -> File.rm_rf!(origin) end)
      git.(["init", "-q"], origin)
      File.write!(Path.join(origin, "README"), "x")
      git.(["add", "."], origin)
      git.(["commit", "-qm", "init"], origin)

      git.(["init", "-q"], root)
      git.(["submodule", "add", "-q", origin, "libs/nested"], root)

      Store.set_root(root)
      Store.put(make_pointer("git-1", "𓀀𓀻𓃉𓏦", file_path: "libs/nested/lib/a.ex"))

      assert Store.get("git-1").file_path == "lib/a.ex"
      assert File.exists?(Path.join([root, "libs", "nested", ".meta", "pointers.yaml"]))
      refute File.exists?(Path.join([root, ".meta", "pointers.yaml"]))
    end
  end

  describe "migrate" do
    test "moves root-store pointers into owning submodule stores", %{root: root} do
      sub = Path.join(root, "sub")
      File.mkdir_p!(Path.join(sub, ".git"))
      File.mkdir_p!(Path.join(root, ".meta"))

      # legacy-style root-store entry pointing into a submodule
      {:ok, yaml} =
        Ymlr.document(%{
          "pointers" => %{
            "legacy-1" => %{
              "token" => "𓀀𓀻𓃉𓏦",
              "file_path" => "sub/lib/legacy.ex",
              "function" => "legacy_fn",
              "description" => "legacy pointer"
            }
          }
        })

      File.write!(Path.join([root, ".meta", "pointers.yaml"]), yaml)

      Store.set_root(root)
      assert Store.get("legacy-1").file_path == "sub/lib/legacy.ex"

      %{moved: moved} = Store.migrate()

      assert moved == 1
      assert Store.get("legacy-1").file_path == "lib/legacy.ex"
      assert File.exists?(Path.join([sub, ".meta", "pointers.yaml"]))

      {:ok, data} = YamlElixir.read_from_file(Path.join([root, ".meta", "pointers.yaml"]))
      refute Map.has_key?(data["pointers"], "legacy-1")
    end
  end

  defp make_pointer(uuid, token, opts \\ []) do
    Pointer.new(%{
      uuid: uuid,
      token: token,
      file_path: Keyword.get(opts, :file_path, "lib/test.ex"),
      class: Keyword.get(opts, :class),
      function: Keyword.get(opts, :function, "login"),
      line: Keyword.get(opts, :line),
      description: Keyword.get(opts, :description, "test pointer")
    })
  end
end
