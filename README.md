# Git2Ex

Precompiled [libgit2](https://libgit2.org/) NIF for Elixir — the git
primitives a code tool's git panel needs, without shelling out to the `git`
CLI and without requiring users to have a Rust toolchain.

预编译的 libgit2 Elixir NIF:为代码工具的 Git 面板提供原语,不依赖 `git`
命令行,使用者也无需 Rust 工具链(编译期自动从 GitHub Releases 下载对应
平台的二进制)。

## Operations

| Function | Purpose |
|---|---|
| `status/1` | Working-tree status (porcelain `XY` codes, staged/unstaged flags, staged-rename detection) |
| `diff_file/3` | Unified diff for one file — `HEAD↔index` (staged) or `index↔worktree` |
| `file_at/3` | Full file content at a revision (`":0"` for the index) — feeds side-by-side diff views |
| `stage/2` `unstage/2` `discard/2` | Stage / unstage / discard one file (unborn-HEAD safe; untracked discard deletes) |
| `commit/2` `commit_amend/2` | Commit the index / amend HEAD |
| `log/2` | Recent commits (`hash`, `author`, `date_unix`, `subject`) |
| `show/2` | A commit's header + patch |

Local repository operations only — the crate is built **without network
transports** (no https/ssh, no openssl dependency).

## Installation

```elixir
def deps do
  [
    {:git2ex, github: "mjason/git2ex"}
  ]
end
```

The matching NIF binary is downloaded from this repository's Releases at
compile time (`RustlerPrecompiled`). Prebuilt targets:

- `x86_64-unknown-linux-gnu` / `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin` / `aarch64-apple-darwin`

To compile from source instead (requires Rust):

```sh
GIT2EX_BUILD=1 mix compile
```

## Example

```elixir
{:ok, %{repo: true, branch: "main", files: files}} = Git2Ex.status("/path/in/repo")
{:ok, true} = Git2Ex.stage("/path/in/repo", "lib/foo.ex")
{:ok, hash} = Git2Ex.commit("/path/in/repo", "add foo")
{:ok, commits} = Git2Ex.log("/path/in/repo", 50)
```

Safety caps: diff/show output 512 KiB, `file_at` 2 MiB (`truncated: true`
past the cap). Binary blobs report `binary: true` with empty content.

## Releasing (maintainers)

1. Bump the version in `mix.exs` **and** `native/git2ex/Cargo.toml`.
2. Tag `vX.Y.Z` and push — the release workflow builds every target and
   attaches the binaries to the GitHub release.
3. `mix rustler_precompiled.download Git2Ex --all --print` and commit the
   regenerated `checksum-Elixir.Git2Ex.exs`.

## License

MIT
