# sweep

Fast cleanup passes for pre-commit, written in Rust on top of
[tree-sitter](https://tree-sitter.github.io/). Python first; the engine is
language-agnostic so more grammars can be added cheaply.

Each rule is one independent, deterministic pass: it parses the file once,
reports diagnostics, and (optionally) carries a fix. `--fix` applies all
non-conflicting fixes and re-checks until nothing is left to do.

> PyPI note: the name `sweep` is taken on PyPI; any future wheel
> distribution will publish as `codesweep` while keeping the `sweep`
> binary and internal naming.

## Rules

| rule | what it does | --fix |
| --- | --- | --- |
| `local-imports` | flags `import`/`from … import` inside functions | hoists into the module import block (stdlib / third-party / first-party section, alphabetical) |
| `docstring-style` | flags docstrings whose sections follow a different convention than configured (reST / Google / NumPy), plus Markdown-style `` `spans` `` in reST docstrings | converts the docstring to the configured convention; doubles backticks |
| `string-annotations` | flags quoted type annotations like `x: "Foo"` | unquotes and inserts `from __future__ import annotations` |

Deliberate exceptions are suppressed with a comment on the same line (or
the line above):

```python
def build():
    from app.models import Model  # sweep: avoid-cycle models imports builders

def load(x: "Config") -> None:  # sweep: ignore[string-annotations] runtime introspection
    ...
```

`# sweep: ignore` (no rule list) silences every rule for that line.

Never auto-fixed, only warned about: conditional imports (`try`/`except
ImportError`, `if TYPE_CHECKING` blocks), relative imports inside
functions, docstrings that can't be converted losslessly.

## Usage

```console
$ sweep check src/            # report
$ sweep check src/ --fix      # apply fixes in place
$ sweep check a.py b.py       # explicit files (what pre-commit passes)
$ sweep check . --select docstring-style
$ sweep rules                 # list rules
```

Exit codes: `0` clean, `1` findings remain, `2` usage/internal error.

## Configuration

`sweep.toml` in the repo root, or `[tool.sweep]` in `pyproject.toml`
(nearest one upward from the working directory wins; `--config PATH`
overrides):

```toml
[tool.sweep]
exclude = ["migrations/"]

[tool.sweep.python]
docstring-style = "rest"      # rest (default) | google | numpy

[tool.sweep.rules.local-imports]
level = "warn"                # warn (default) | error | off
fix = "hoist"                 # hoist (default) | "off" — whether --fix moves imports
known-first-party = ["mypkg"]

[tool.sweep.rules.docstring-style]
level = "warn"

[tool.sweep.rules.string-annotations]
level = "warn"
```

First-party packages for import-section placement are also picked up
automatically from `[project].name`, `[tool.poetry].name`,
`[tool.ruff.lint.isort].known-first-party` and
`[tool.isort].known_first_party`.

Blank lines between import sections are not managed; run ruff/isort
formatting after `--fix` if you care about exact spacing.

## pre-commit

```yaml
repos:
  - repo: <this repo>
    rev: v0.1.0
    hooks:
      - id: sweep        # check only
      # or:
      - id: sweep-fix    # check + fix in place
```

The hook builds with cargo on first install (`language: rust`) and is
cached by pre-commit afterwards.

## Development

```console
$ cargo test          # unit + fixture round-trip tests
$ cargo run -- check tests/fixtures/hoist
```

Fixtures under `tests/fixtures/<name>/` are `{sweep.toml|pyproject.toml,
input.py, expected.py}`; the CLI test copies them to a temp dir, runs
`check --fix`, compares with `expected.py`, and re-runs to prove
idempotency.

Adding a rule: implement `engine::rule::Rule` (one struct, one `check`),
register it in `langs/python/rules/mod.rs`, add a fixture. Adding a
language: add the grammar crate, a `langs/<lang>/` module with its own
rules, and dispatch by extension in `main.rs`/`runner.rs`.
