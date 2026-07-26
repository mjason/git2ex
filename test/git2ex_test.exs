defmodule Git2ExTest do
  # Full lifecycle against real throwaway repositories. Requires the NIF
  # (run with GIT2EX_BUILD=1 locally, or after downloading a release binary).
  use ExUnit.Case, async: true

  @moduletag :tmp_dir

  setup %{tmp_dir: dir} do
    {_, 0} = System.cmd("git", ["-C", dir, "init", "-q", "-b", "main"])
    {_, 0} = System.cmd("git", ["-C", dir, "config", "user.email", "t@example.com"])
    {_, 0} = System.cmd("git", ["-C", dir, "config", "user.name", "Test"])
    %{repo: dir}
  end

  defp trimmed_files({:ok, %{files: files}}),
    do: Enum.map(files, &%{&1 | status: String.trim(&1.status)})

  test "a plain directory reports repo: false" do
    outside = Path.join(System.tmp_dir!(), "git2ex-not-repo-#{System.unique_integer([:positive])}")
    File.mkdir_p!(outside)
    on_exit(fn -> File.rm_rf(outside) end)

    assert {:ok, %{repo: false, root: nil, files: []}} = Git2Ex.status(outside)
  end

  test "discover finds the repo root from a subdirectory; nil outside a repo", %{repo: repo} do
    sub = Path.join(repo, "a/b")
    File.mkdir_p!(sub)

    assert {:ok, %{repo: true, root: root}} = Git2Ex.discover(sub)
    # libgit2 may return a trailing-slash-normalized, symlink-resolved path.
    assert Path.expand(root) == Path.expand(repo)

    outside = Path.join(System.tmp_dir!(), "git2ex-none-#{System.unique_integer([:positive])}")
    File.mkdir_p!(outside)
    on_exit(fn -> File.rm_rf(outside) end)
    assert {:ok, %{repo: false, root: nil}} = Git2Ex.discover(outside)
  end

  test "status → stage → commit → log → show lifecycle", %{repo: repo} do
    File.write!(Path.join(repo, "a.txt"), "hello\n")

    {:ok, %{repo: true, branch: "main"}} = Git2Ex.status(repo)
    assert [%{path: "a.txt", status: "??", staged: false, unstaged: true}] =
             trimmed_files(Git2Ex.status(repo))

    {:ok, true} = Git2Ex.stage(repo, "a.txt")
    assert [%{path: "a.txt", status: "A", staged: true, unstaged: false}] =
             trimmed_files(Git2Ex.status(repo))

    {:ok, hash} = Git2Ex.commit(repo, "add a")
    assert hash =~ ~r/^[0-9a-f]{7}$/

    {:ok, [%{subject: "add a", author: "Test", hash: ^hash, date_unix: ts}]} =
      Git2Ex.log(repo, 50)

    assert is_integer(ts)

    {:ok, %{text: text, truncated: false}} = Git2Ex.show(repo, hash)
    assert text =~ "add a" and text =~ "+hello"

    assert {:error, _} = Git2Ex.commit(repo, "nothing staged")
  end

  test "MM file is staged AND unstaged; diffs render per side", %{repo: repo} do
    File.write!(Path.join(repo, "b.txt"), "one\n")
    {:ok, true} = Git2Ex.stage(repo, "b.txt")
    {:ok, _} = Git2Ex.commit(repo, "base")

    File.write!(Path.join(repo, "b.txt"), "one\ntwo\n")
    {:ok, true} = Git2Ex.stage(repo, "b.txt")
    File.write!(Path.join(repo, "b.txt"), "one\ntwo\nthree\n")

    assert [%{status: "MM", staged: true, unstaged: true}] = trimmed_files(Git2Ex.status(repo))

    {:ok, %{diff: staged_diff}} = Git2Ex.diff_file(repo, "b.txt", true)
    assert staged_diff =~ "+two" and not (staged_diff =~ "+three")

    {:ok, %{diff: unstaged_diff}} = Git2Ex.diff_file(repo, "b.txt", false)
    assert unstaged_diff =~ "+three" and not (unstaged_diff =~ "+two")
  end

  test "file_at serves index/HEAD blobs; missing is not an error", %{repo: repo} do
    File.write!(Path.join(repo, "d.txt"), "v1\n")
    {:ok, true} = Git2Ex.stage(repo, "d.txt")
    {:ok, _} = Git2Ex.commit(repo, "v1")

    File.write!(Path.join(repo, "d.txt"), "v2\n")
    {:ok, true} = Git2Ex.stage(repo, "d.txt")

    assert {:ok, %{content: "v1\n", missing: false}} = Git2Ex.file_at(repo, "HEAD", "d.txt")
    assert {:ok, %{content: "v2\n", missing: false}} = Git2Ex.file_at(repo, ":0", "d.txt")
    assert {:ok, %{missing: true}} = Git2Ex.file_at(repo, "HEAD", "never.txt")
  end

  test "unstage, discard (tracked restore + untracked delete)", %{repo: repo} do
    File.write!(Path.join(repo, "e.txt"), "keep\n")
    {:ok, true} = Git2Ex.stage(repo, "e.txt")
    {:ok, _} = Git2Ex.commit(repo, "base")

    File.write!(Path.join(repo, "e.txt"), "dirty\n")
    {:ok, true} = Git2Ex.discard(repo, "e.txt")
    assert File.read!(Path.join(repo, "e.txt")) == "keep\n"

    File.write!(Path.join(repo, "f.txt"), "x\n")
    {:ok, true} = Git2Ex.stage(repo, "f.txt")
    {:ok, true} = Git2Ex.unstage(repo, "f.txt")
    assert [%{path: "f.txt", status: "??"}] = trimmed_files(Git2Ex.status(repo))

    {:ok, true} = Git2Ex.discard(repo, "f.txt")
    refute File.exists?(Path.join(repo, "f.txt"))
  end

  test "staged renames are detected and listed under the new path", %{repo: repo} do
    File.write!(Path.join(repo, "old.txt"), "content\n")
    {:ok, true} = Git2Ex.stage(repo, "old.txt")
    {:ok, _} = Git2Ex.commit(repo, "base")

    {_, 0} = System.cmd("git", ["-C", repo, "mv", "old.txt", "renamed.txt"])

    files = trimmed_files(Git2Ex.status(repo))
    assert %{status: "R", staged: true} = Enum.find(files, &(&1.path == "renamed.txt"))
    refute Enum.any?(files, &(&1.path == "old.txt"))
  end

  test "amend rewrites the tip", %{repo: repo} do
    File.write!(Path.join(repo, "g.txt"), "x\n")
    {:ok, true} = Git2Ex.stage(repo, "g.txt")
    {:ok, _} = Git2Ex.commit(repo, "typo mesage")
    {:ok, _} = Git2Ex.commit_amend(repo, "fixed message")

    {:ok, [%{subject: "fixed message"}]} = Git2Ex.log(repo, 50)
  end

  test "empty repository: unborn branch name, empty log, unstage fallback", %{repo: repo} do
    {:ok, %{branch: "main"}} = Git2Ex.status(repo)
    assert {:ok, []} = Git2Ex.log(repo, 10)

    File.write!(Path.join(repo, "h.txt"), "x\n")
    {:ok, true} = Git2Ex.stage(repo, "h.txt")
    {:ok, true} = Git2Ex.unstage(repo, "h.txt")
    assert [%{path: "h.txt", status: "??"}] = trimmed_files(Git2Ex.status(repo))
  end
end
