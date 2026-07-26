defmodule Git2Ex.MixProject do
  use Mix.Project

  @version "0.2.0"
  @source_url "https://github.com/mjason/git2ex"

  def project do
    [
      app: :git2ex,
      version: @version,
      elixir: "~> 1.15",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description: "Precompiled libgit2 NIF for Elixir: status, diff, stage, commit, log, show.",
      package: package(),
      source_url: @source_url,
      docs: [main: "Git2Ex", extras: ["README.md"]]
    ]
  end

  def application do
    [extra_applications: [:logger]]
  end

  defp deps do
    [
      {:rustler_precompiled, "~> 0.8"},
      # Only needed when building the NIF locally (GIT2EX_BUILD=1) or in CI.
      {:rustler, "~> 0.36", optional: true},
      {:ex_doc, ">= 0.0.0", only: :dev, runtime: false}
    ]
  end

  defp package do
    [
      files: [
        "lib",
        "native/git2ex/src",
        "native/git2ex/Cargo.toml",
        "native/git2ex/Cargo.lock",
        "checksum-*.exs",
        "mix.exs",
        "README.md",
        "LICENSE"
      ],
      licenses: ["MIT"],
      links: %{"GitHub" => @source_url}
    ]
  end
end
