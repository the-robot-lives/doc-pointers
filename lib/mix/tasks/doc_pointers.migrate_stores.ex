defmodule Mix.Tasks.DocPointers.MigrateStores do
  @shortdoc "Re-split pointers into their owning submodule .meta/pointers.yaml"

  @moduledoc """
  Re-resolves every loaded pointer's owning store (nearest git repo at or below
  the root) and rewrites affected `.meta/pointers.yaml` files. Legacy monorepo-root
  entries whose file lives inside a submodule move into that submodule's store;
  genuinely root-level entries stay in `<root>/.meta/pointers.yaml`.

  Usage:

      mix doc_pointers.migrate_stores [--root PATH]
  """

  use Mix.Task

  alias DocPointers.Store

  @requirements ["app.start"]

  @impl Mix.Task
  def run(args) do
    {opts, _rest, _invalid} = OptionParser.parse(args, strict: [root: :string])

    root = opts[:root] || File.cwd!()
    Store.set_root(root)

    %{moved: moved, stores: stores} = Store.migrate()

    Mix.shell().info("migrated #{moved} pointer(s) across #{length(stores)} store(s):")
    Enum.each(stores, &Mix.shell().info("  #{&1 || "."}/.meta/pointers.yaml"))
  end
end
