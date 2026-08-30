# Contributing to Oxidal

Issues and pull requests are welcome. Bug reports with the escape sequences or the session kind that triggered them are especially useful, since terminal emulation has a long tail of edge cases and the fastest way to fix one is to reproduce it.

## Getting set up

Requirements and build instructions live in the [README](README.md#getting-started). Short version: Rust 1.85 or newer, then `cargo run --release`. The first build pulls GPUI from the Zed repository and compiles a large dependency tree, so expect it to take a while.

Before opening a pull request:

```sh
cargo fmt
cargo clippy --all-targets
cargo build --release
```

CI additionally runs `cargo audit --deny warnings` on every push to `main`, so a pull request that introduces an advisory-flagged dependency will fail there.

## Branching

Work happens on `dev`. Branch off it, name the branch after what it does, and open the pull request back into `dev`:

```
feat/monitoring-connections
fix/terminal-scroll
```

`main` is the release branch. Merging `dev` into `main` triggers the build and publish workflow, which produces installers for all six targets and cuts a GitHub release. Don't push to `main` directly.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): description
```

This matters more than style here: `.github/workflows/publish.yml` parses every commit subject to build the release notes, so the prefix you write decides which section your change appears under. A commit that doesn't parse lands in "Other changes".

### Sections

| Section | Triggers on | Example |
|---|---|---|
| 💥 Breaking changes | any type with `!` before the colon | `feat(ssh)!: drop support for DSA host keys` |
| 🚀 Features | type `feat`, `feature`, `feats` | `feat(proxy): added socks5 proxy implementation` |
| 🐛 Bug fixes | type `fix`, `fixes`, `hotfix`, `bugfix`, `bug` | `fix(ssh): host key prompt could stay hidden` |
| 🔐 Security | scope containing `security`, scope `hardening`, or type `security`/`sec` | `fix(security): harden updater and paste handling` |
| ⚡ Performance | type `perf`, `performance`, `optim` | `perf(editor): reduce allocations in scrollback` |
| 📦 Dependencies | scope `deps`/`dep`/`dependencies`, type `deps`/`dep`, or a bare `Bump …`/`Upgrade …` | `chores(deps): Bump sha2 version` |
| 📚 Documentation | type `docs`/`doc`, or a prefix-less subject mentioning README, LICENSE or doc | `docs: document the SOCKS5 proxy settings` |
| 🏗️ Build & CI | scope `ci`/`cd`/`workflow`/`workflows`/`actions`/`release`, or type `ci`/`cd`/`build`/`packaging`/`workflow` | `feat(workflow): added security deps check` |
| 🧹 Maintenance | type `chore`, `chores`, `refactor`, `style`, `test`, `tests`, `cleanup` | `chores(cleanup): removed dead code` |
| 🔀 Other changes | anything that doesn't match the above | `Convert features list to a table format` |

Two more sections are added automatically and aren't driven by commit subjects: **👥 Contributors**, built from the unique author names in the range with `[bot]` accounts filtered out, and **📥 Downloads**, which carries the installer list, the macOS quarantine note and the SHA256 checksums.

### How a commit is routed

First match wins:

1. **Version bumps are dropped.** `Bump 0.5.1`, `Update v0.5.1`, `Release v0.5.1`, `update version 0.2.1` and `version 0.2.1` never appear, and don't count toward the commit total in the header.
2. **`!` wins over everything.** `feat(ssh)!: …` goes to Breaking changes, not Features.
3. **Scope beats type.** `fix(security):` is filed under Security rather than Bug fixes, and `feat(ci):` under Build & CI rather than Features. Only the `security`, `deps` and CI-family scopes do this; every other scope defers to the type.
4. **Then type**, per the table above.
5. **Then a fallback** for subjects with no `type:` prefix, which sends `Bump …`/`Upgrade …` to Dependencies and anything mentioning README, LICENSE or doc to Documentation.

Type and scope are matched case-insensitively, so `Feat(UI):` behaves like `feat(ui):`.

### Notes

Scope is optional but worth writing, since it's rendered as a bold prefix in the notes:

```
- **ssh**: allow multiple signature per host ([`10eb023`](https://github.com/sh4den/Oxidal/commit/10eb023))
```

Multiple scopes are fine — `feat(editor,client): optimized memory allocation` renders the pair verbatim and routes on its type.

Two asymmetries to know about:

- `feat(packaging):` is filed under **Features**, because `packaging` is a routing type but not a routing scope. That's intended: packaging work that users can feel, like adding Hyprland compatibility, belongs in the feature list rather than buried under Build & CI.
- `chores(cargo): bump version to 0.3.1` is filed under **Maintenance** rather than dropped. The skip patterns match the raw subject, so they only catch bare version bumps, not ones wearing a conventional prefix.

## Releases

Releases are cut from `main`, and the version comes from `Cargo.toml` rather than the tag. To publish:

1. Bump `version` in `Cargo.toml` on `dev`.
2. Open a pull request from `dev` into `main` and merge it.

The workflow then builds all six targets, generates the release notes from the commits since the previous tag, and publishes the tag `v<version>` with the installers, portable builds and `SHA256SUMS` attached. Tagging by hand isn't necessary.

## License

Oxidal is licensed under the [GNU GPL v3](LICENSE). Contributions are accepted under the same license.
