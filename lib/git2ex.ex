defmodule Git2Ex do
  @moduledoc """
  Precompiled libgit2 bindings for Elixir — the git primitives a code tool's
  git panel needs: `status/1`, `diff_file/3`, `file_at/3`, `stage/2`,
  `unstage/2`, `discard/2`, `commit/2`, `commit_amend/2`, `log/2`, `show/2`.

  Local repository operations only; the native crate is built without
  network transports (no https/ssh, no openssl).

  Binaries are downloaded from GitHub releases at compile time via
  `RustlerPrecompiled` — consumers never need a Rust toolchain. Set
  `GIT2EX_BUILD=1` (with Rust installed) to compile from source instead.

  All fallible functions return `{:ok, value}` / `{:error, message}`.
  Sizes are capped in the NIF (diff/show 512 KiB, file_at 2 MiB) with a
  `truncated` flag.
  """

  version = Mix.Project.config()[:version]

  use RustlerPrecompiled,
    otp_app: :git2ex,
    crate: "git2ex",
    base_url: "https://github.com/mjason/git2ex/releases/download/v#{version}",
    force_build: System.get_env("GIT2EX_BUILD") in ["1", "true"],
    targets: ~w(
      aarch64-apple-darwin
      aarch64-unknown-linux-gnu
      x86_64-apple-darwin
      x86_64-unknown-linux-gnu
    ),
    nif_versions: ["2.15"],
    version: version

  @typedoc "One changed file, porcelain-style."
  @type file_status :: %{
          path: String.t(),
          status: String.t(),
          staged: boolean(),
          unstaged: boolean()
        }

  @doc """
  Working-tree status. `{:ok, %{repo, root, branch, files}}`; `repo: false`
  when `path` is not inside a repository. Status codes are porcelain `XY`
  (may carry a trailing space, e.g. `"M "`); staged renames are detected
  and listed under the NEW path.
  """
  def status(_path), do: err()

  @doc "Unified diff for one file. `staged?` picks HEAD↔index, else index↔worktree."
  def diff_file(_path, _file, _staged), do: err()

  @doc ~S(File content at a revision — `":0"` for the index, or any commit-ish. Missing paths report `missing: true`.)
  def file_at(_path, _rev, _file), do: err()

  @doc "Stage one file (`git add`)."
  def stage(_path, _file), do: err()

  @doc "Unstage one file (unborn-HEAD safe)."
  def unstage(_path, _file), do: err()

  @doc "Discard a file's working-tree changes; untracked files are deleted."
  def discard(_path, _file), do: err()

  @doc "Commit the index. Returns the short hash. Refuses empty commits."
  def commit(_path, _message), do: err()

  @doc "Amend HEAD with the current index; an empty message keeps the original."
  def commit_amend(_path, _message), do: err()

  @doc "Recent commits: `{:ok, [%{hash, author, date_unix, subject}]}`."
  def log(_path, _limit), do: err()

  @doc "A commit's header + patch as text."
  def show(_path, _hash), do: err()

  defp err, do: :erlang.nif_error(:nif_not_loaded)
end
