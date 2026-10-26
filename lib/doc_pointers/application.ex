defmodule DocPointers.Application do
  @moduledoc """
  〚📦:352a2da5-7321-5a0b-9ff1-bdc47e6a0c39〛 DocPointers.Application :: OTP application entry: boots the store GenServer rooted at DOC_POINTERS_ROOT or cwd.
  """
  use Application

  @impl true
  def start(_type, _args) do
    root =
      Application.get_env(:doc_pointers, :boot_root) ||
        System.get_env("DOC_POINTERS_ROOT") ||
        Application.get_env(:doc_pointers, :root) ||
        File.cwd!()

    children = [
      {DocPointers.Store, root: root}
    ]

    Supervisor.start_link(children, strategy: :one_for_one, name: DocPointers.Supervisor)
  end
end
