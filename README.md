# sweep

Fast, deterministic cleanup passes for pre-commit, written in Rust on top
of [tree-sitter](https://tree-sitter.github.io/). Python first; the engine
is language-agnostic so more grammars can be added cheaply.

Each rule is one independent pass: it parses the file once, reports
diagnostics, and (optionally) carries a fix. `--fix` applies all
non-conflicting fixes and re-checks until nothing is left to do.

## Getting started

### As a pre-commit hook

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/hmsgit/sweep
    rev: v0.1.0b2
    hooks:
      - id: sweep        # check only
      # or:
      - id: sweep-fix    # check + fix in place
```

The hook builds with cargo on first install (`language: rust`) and is
cached by pre-commit afterwards. A Rust toolchain is required once, on
first install.

`sweep-fix` is just `sweep` with `--fix` baked in; any CLI flag can
also be passed through pre-commit's `args`, since pre-commit runs
`entry + args + filenames`:

```yaml
      - id: sweep
        args: [--fix]                      # same as sweep-fix
      - id: sweep
        args: [--select, imports-ban-local]    # run a single rule
```

### As a CLI

```console
$ pip install codesweep       # installs the `sweep` binary (see Naming)
$ # or from source:
$ cargo install --path .
$ sweep check src/            # report findings
$ sweep check src/ --fix      # apply fixes in place
$ sweep rules                 # list rules
```

Most repos need no configuration: defaults are reST docstrings, error
levels for the three main rules (info for line length), and line length
from ruff's config when present.

### Developing

```console
$ cargo test                  # unit + fixture round-trip tests
$ cargo clippy --all-targets && cargo fmt --check
$ cargo run -- check tests/fixtures/hoist
```

See [Extending](#extending) for how to add a rule or a language.

---

## Rules

| rule | detects | `--fix` |
| --- | --- | --- |
| [`imports-ban-local`](#imports-ban-local) | `import` / `from … import` inside functions | hoists to the module import block, section-sorted |
| [`docstring-style`](#docstring-style) | docstrings following a different convention than configured; wrong inline markup | converts to the configured convention; fixes markup |
| [`string-annotations`](#string-annotations) | quoted type annotations like `x: "Foo"` | unquotes; inserts `from __future__ import annotations` |
| [`docstring-start`](#docstring-start) | multi-line docstrings whose content starts on the opening-quote line | moves the content to the next line, aligned with the quotes |
| [`docstring-line-length`](#docstring-line-length) | docstring lines exceeding the line length | `info` by default (report only); at `warn`/`error` re-flows prose |

**House-style rules** — opt-in, `off` by default (see
[House-style rules](#house-style-rules)):

| rule | detects | `--fix` |
| --- | --- | --- |
| `dict-style` | dicts built contrary to the configured form (`literal` or `function`) | rewrites in the configured direction |
| `annotate-module-const` | UPPER_CASE module constants without a `Final` annotation | adds `: Final` / wraps as `Final[T]`, inserts the typing import |
| `casing-enum-key` | enum member names not in the configured case | warn-only (cross-file rename) |
| `casing-enum-val` | enum string values not in the configured case | warn-only (changes serialized data) |
| `casing-module-const` | module constant names not in the configured case | warn-only (cross-file rename) |
| `no-emoji` | any emoji/unicode icon (pictographs, ✓/✗, arrows, shapes) not in the allowed set — enabled by setting `allowed-emojis` | deletes in comments/docstrings; warn-only inside strings |

### imports-ban-local

Imports belong at module level. Function-level imports usually exist for
one of two reasons — breaking an import cycle, or deferring a heavy/optional
dependency — and both deserve to be visible and justified:

```python
def build():
    from app.models import Model  # sweep: avoid-cycle models imports builders
```

Everything unjustified is flagged. Under `--fix` (at the default
`error` level, or `warn`) the import is moved into the module's top
import region:

- **Sections** follow the common isort layout: `__future__`, standard
  library, third-party, first-party, relative. Section membership comes
  from an embedded copy of `sys.stdlib_module_names` plus the configured
  or discovered first-party package names.
- **Position** within the section is alphabetical by dotted module path
  (case-insensitive).
- If an **identical import** already exists at top level, the local copy
  is simply removed.
- If the import was the **only statement** in its block, it is replaced
  with `pass` to keep the code valid.

Warned about but **never auto-hoisted** (the fix would change behavior):

- imports under `try` / `if` / `with` / loops inside the function
  (e.g. `try: import orjson / except ImportError`),
- relative imports inside functions (almost always cycle dodges),
- import lines sharing the line with other code or a trailing comment.

Blank lines between import sections are not managed; run ruff/isort
formatting after `--fix` if you care about exact spacing.

### docstring-style

Enforces one docstring convention across the project: reST (Sphinx field
lists), Google, or NumPy. Detection is based on section signatures —
`Args:`/`Returns:` headers (Google), dash-underlined `Parameters` headers
(NumPy), `:param x:` field lists (reST). Plain-prose docstrings with no
sections match any convention and are never flagged.

Under `--fix`, mismatched docstrings are converted through a
style-neutral IR. Supported fields and their mappings:

| IR | reST | Google | NumPy |
| --- | --- | --- | --- |
| params | `:param x:` + `:type x:` | `Args:` — `x (int): …` | `Parameters` — `x : int` |
| returns | `:returns:` + `:rtype:` | `Returns:` — `int: …` | `Returns` — `int` + desc |
| yields | `:yields:` + `:ytype:` | `Yields:` | `Yields` |
| raises | `:raises X:` | `Raises:` — `X: …` | `Raises` — `X` + desc |
| attributes | `:ivar x:` + `:vartype x:` | `Attributes:` | `Attributes` |
| extras | kept verbatim | `Examples:` etc., kept verbatim | header + dashes, kept verbatim |

Conversion is **lossless or not at all**: anything the parser can't
represent faithfully (unknown reST fields, directives, flush-left prose
after fields, multiple NumPy return entries, f-string/concatenated
docstrings, non-triple quotes that would need to become multi-line)
downgrades the finding to warn-only. Summary and description prose,
multi-paragraph descriptions and per-field continuation lines survive
the round trip.

**Inline markup**: when the convention is reST, the house style is
single-backtick spans — ``double-backtick`` reST literals are flagged
and fixed down to `single`. (Strict-reST purists note: this is a
deliberate house-style choice, not textbook reST.) Roles like
:func:`name` and doctest lines are left alone; no markup check runs
for Google/NumPy conventions.

### string-annotations

Quoted "forward reference" annotations predate PEP 563; with
`from __future__ import annotations` every annotation is lazy and the
quotes are noise:

```python
def fetch(item: "Item") -> "list[Item]": ...
# becomes
from __future__ import annotations
def fetch(item: Item) -> list[Item]: ...
```

The fix unquotes the annotation and inserts the future import (once,
after the module docstring) if missing. Strings that are **values**, not
forward references, are never touched:

- contents of `Literal[...]` (any nesting, `typing.Literal` included),
- metadata arguments of `Annotated[T, ...]` (the first element is a
  type and *is* unquoted),
- arguments of calls inside annotations,
- f-strings, concatenated and multi-line strings.

Caveat: code that inspects annotations at runtime with
`typing.get_type_hints()` behaves identically, but code reading
`__annotations__` raw will see strings after the future import lands —
that is PEP 563 semantics, not a sweep quirk. Suppress per line if you
depend on eager annotations.

### docstring-start

Multi-line docstrings start their content on the line *after* the
opening quotes, aligned with them (pydocstyle's D213 shape):

```python
def emit(scope):
    """
    Emit a change event for consumers.

    :param scope: tenant scope of the event.
    """
```

Single-line docstrings stay inline. Closing quotes are never touched —
they may share the last content line or sit on their own line, whichever
the author wrote. Docstring rewrites from other rules (conversion,
rewrap) emit this shape directly.

### docstring-line-length

Reports every docstring line (quotes and indentation included) that
exceeds the configured line length. Code lines are ruff's business
(`E501`); this rule only measures docstrings.

The default level is `info`: report only, never rewritten, never fails
the run. Opt into rewriting by raising the level:

```toml
[tool.sweep.rules.docstring-line-length]
level = "warn"   # or "error"
```

Then `--fix` re-flows docstring **prose** — greedy word wrap, paragraph
boundaries preserved, budgeting the base indentation and the opening
quotes on the first line. With re-flow enabled, `docstring-style`
conversions wrap their output too, so a Google→reST conversion lands
within the limit in one pass.

Never re-flowed: bullet lists, numbered lists, doctest lines, reST
directives, and `::` literal-block introducers. A line that cannot be
shortened (one long word, a URL) keeps its warning and is left alone.

## House-style rules

The core rules above are on by default; these six encode house
conventions and stay `off` until a project opts in:

```toml
[tool.sweep.python]
allowed-emojis = ""              # presence enables no-emoji; "" = no exceptions

[tool.sweep.rules]
dict-style = "func"           # literal | function/func (shorthand enables at warn)
annotate-module-const = "warn"
casing-enum-key = "lower"        # lower | upper (shorthand enables at warn)
casing-enum-val = "lower"
casing-module-const = "lower"
```

Notes:

- `dict-style` converts only what it can express faithfully: non-string or
  non-identifier keys, Python keywords, duplicate keys, or comments
  inside the literal. Splats pass through (`{**a, "b": 1}` →
  `dict(**a, b=1)`).
- `annotate-module-const` only annotates; whether the *name* should be
  `UPPER_CASE` or `lower_case` is `casing-module-const`'s business —
  the two are independent knobs.
- The casing rules never autofix: renaming an identifier safely needs
  cross-file refactoring, and changing an enum's string *value* changes
  serialized data. They warn; a human renames.
- Constants are recognized by SCREAMING_CASE or an existing `Final`
  annotation; plain lowercase module assignments are indistinguishable
  from module state and are never flagged.
- Casing rules take a table form too:
  `casing-module-const = { level = "error", case = "upper" }`.
- `no-emoji` has a single knob: `[tool.sweep.python] allowed-emojis`.
  Its presence enables the rule (at warn); its value is the exception
  list (`""` = flag every emoji/icon). Detected: emoji blocks, dingbats
  (✓/✗), arrows (→), misc technical and geometric-shape characters;
  invisible emoji plumbing (variation selectors, ZWJ) is cleaned up
  with its base character but never flagged alone.

## Severity levels

Every rule has one knob, `level`, and it decides everything:

| level | shown | `--fix` rewrites | fails the run |
| --- | --- | --- | --- |
| `off` | no | no | no |
| `info` | yes | **no** — purely informational | no |
| `warn` | yes | yes | only with `--strict` |
| `error` | yes | yes | **yes** |

Defaults: `imports-ban-local`, `docstring-style` and `string-annotations`
are `error`; `docstring-line-length` is `info`. Relax rules to `warn`
(fixed but not gating) or `info` (notify only) per project.

pre-commit normally hides the output of **passing** hooks, which would
make info/warn findings invisible. The sweep hooks therefore ship with
`verbose: true`, and sweep prints nothing when a piped run is clean —
so commits with findings show them even when the hook passes, and
clean commits stay quiet.

## Suppressing findings

A directive comment on the flagged line or the line directly above:

```python
def build():
    from app.models import Model  # sweep: avoid-cycle models imports builders

def load(x: "Config") -> None:  # sweep: ignore[string-annotations] runtime introspection
    ...

# sweep: ignore
anything_on_this_line_is_exempt()
```

Grammar:

- `# sweep: ignore` — silence every rule for the line.
- `# sweep: ignore[rule-a, rule-b] <optional reason>` — silence specific rules.
- `# sweep: avoid-cycle <optional reason>` — shorthand for
  `ignore[imports-ban-local]` with cycle-avoidance as the stated reason.

Reasons are free text and encouraged; they are for the next reader, not
for the tool.

Blanket markers from other tools are honored too: a bare `# noqa` or a
bare `# type: ignore` on a line suppresses sweep findings on that line
(same-line only, matching flake8/mypy semantics — no line-above reach).
Code-carrying forms (`# noqa: F401`, `# type: ignore[union-attr]`) name
that tool's rules, say nothing about sweep, and don't suppress.

## Configuration

Configuration lives in `sweep.toml` or `[tool.sweep]` inside
`pyproject.toml`. Discovery is **per file**: each checked file uses the
nearest config found walking up from its own directory (`sweep.toml`
beats `pyproject.toml` at the same level). This makes monorepos work
out of the box — pre-commit config at the repo root, one
`app/*/pyproject.toml` per app, and every file is judged by its own
app's settings. `--config PATH` overrides discovery for all files.
Everything is optional:

```toml
[tool.sweep]
exclude = ["migrations/"]     # path substrings to skip when walking directories
line-length = 79              # falls back to [tool.ruff].line-length, then 79

[tool.sweep.python]
docstring-style = "rest"      # rest (default) | google | numpy
# allowed-emojis = "→✓"       # set to enable no-emoji; the value is the
                              # exception list ("" = flag every emoji/icon)

[tool.sweep.rules.imports-ban-local]
level = "error"               # off | info | warn | error (default: error)
known-first-party = ["mypkg"]

[tool.sweep.rules.docstring-style]
level = "error"               # default: error

[tool.sweep.rules.docstring-start]
level = "error"               # default: error

[tool.sweep.rules.string-annotations]
level = "error"               # default: error

[tool.sweep.rules.docstring-line-length]
level = "info"                # default: info — report only; warn/error enable re-flow
```

When a rule needs nothing but a level, the bare shorthand keeps it to
one line each:

```toml
[tool.sweep.rules]
docstring-style = "warn"
imports-ban-local = "warn"
string-annotations = "warn"
```

Values discovered automatically from `pyproject.toml`, so most projects
need no `[tool.sweep]` section at all:

- **first-party packages**: `[project].name`, `[tool.poetry].name`,
  `[tool.ruff.lint.isort].known-first-party`, `[tool.isort].known_first_party`;
- **line length**: `[tool.ruff].line-length`.

See [Severity levels](#severity-levels) for what each level does.

## CLI reference

```
sweep check [PATHS]... [--fix] [--strict] [--output-format FMT] [--term MODE]
            [--select RULES] [--ignore RULES] [--config PATH]
sweep rules
```

- `PATHS` — files and/or directories (default `.`). Directories are
  walked recursively for supported files, honoring `.gitignore` and the
  `exclude` config. Explicitly passed files (what pre-commit does) are
  always checked, excludes notwithstanding.
- `--fix` — apply available fixes in place. Fixes within one run are
  applied together when they don't conflict; conflicting ones are picked
  up on a re-check, up to a bounded number of rounds.
- `--strict` — treat warnings as errors for the exit code (gate CI
  without touching config).
- `--select` / `--ignore` — comma-separated rule names to run / skip.
- `--output-format full|concise` — `full` (default) renders one block
  per finding with the source snippet; `concise` prints exactly one
  line per finding (the header only), handy for greps, logs and dense
  pre-commit output.
- `--term auto|plain|color|hyper` — terminal output control. `auto`
  (default) colors when stdout is a TTY (`NO_COLOR` respected) and adds
  OSC 8 hyperlinks on the `path:line:col` location when the terminal is
  known to render them (iTerm2, WezTerm, kitty, VS Code, ghostty, VTE,
  Konsole). `plain` strips everything; `color`/`hyper` force it on.

Findings render ruff-style — location, severity, rule, message, the
offending line with a caret underline, and `[*]` marking fixable:

```
app/models.py:21:5: error[imports-ban-local] `import json` inside a function; hoist to module level or mark it `# sweep: avoid-cycle` [*]
   |
21 |     import json
   |     ^^^^^^^^^^^
   |

Found 3 issue(s) (2 error(s), 1 info).
[*] 2 fixable with the `--fix` option.
```

Exit codes: `0` clean or only info/warn findings, `1` error findings
remain (warnings too under `--strict`), `2` usage or internal error.

Files are processed in parallel (rayon); a few hundred files check in
well under a second.

## Fix semantics

Fixes are byte-range edits. Per file and per round, sweep applies every
fix whose edits don't overlap an already-accepted edit, then re-parses
and re-checks. The loop ends when a round changes nothing, or after 10
rounds. Consequences:

- fixes are **idempotent** — running `--fix` twice never changes the
  file twice (the test suite enforces `fix(fix(x)) == fix(x)`),
- two rules editing the same region (e.g. a style conversion and a
  rewrap of the same docstring) resolve over consecutive rounds instead
  of clobbering each other,
- a fix that cannot actually change anything is never offered, so
  unfixable findings simply remain as warnings.

## Extending

**A new rule** is one struct implementing `engine::rule::Rule`
(`src/engine/rule.rs`):

```rust
pub trait Rule: Send + Sync {
    fn name(&self) -> &'static str;    // kebab-case id used everywhere
    fn explain(&self) -> &'static str; // one-liner for `sweep rules`
    fn check(&self, ctx: &FileContext) -> Vec<Diagnostic>;
}
```

Rules never mutate; they return diagnostics with optional fixes.
Register it in `langs/python/rules/mod.rs`, add a fixture directory
under `tests/fixtures/<name>/` with a config, `input.py` and
`expected.py` — the harness runs the real binary against it, compares
output, and re-runs to prove idempotency.

**A new language**: add the tree-sitter grammar crate, create
`src/langs/<lang>/` with its own rules module, and dispatch by file
extension in `main.rs` / `engine/runner.rs`. The engine (diagnostics,
fixes, runner, suppression comments, config) is language-agnostic.

## Releasing

Cargo.toml carries the SemVer version (`0.1.0-beta.1`); PyPI and git
tags use the PEP 440 spelling (`0.1.0b1` / `v0.1.0b1`), which maturin
derives automatically. `scripts/bump.py` owns the mapping and the bump
logic:

```console
$ scripts/bump.py --beta          # 0.1.0 → 0.1.0-beta.1  (or beta.N → beta.N+1)
$ scripts/bump.py --rc            # beta.N → rc.1          (or rc.N → rc.N+1)
$ scripts/bump.py                 # rc.N → final           (strips the pre-release)
$ scripts/bump.py minor --beta    # 0.1.x → 0.2.0-beta.1
$ scripts/bump.py patch --git     # bump + commit + tag; then: git push --follow-tags
```

Pushing a `v*` tag triggers the release workflow (wheels + sdist →
PyPI). The same thing is available in the GitHub UI as the `bump`
workflow (Actions → bump → Run workflow, pick level and channel);
it commits, tags, and dispatches the release for you. Published
versions are immutable on PyPI — never move a tag that has released.

## Naming

Internally everything is `sweep` — repo, crate, binary, config tables,
suppression comments. The name `sweep` is taken on PyPI, so the wheel
publishes as **`codesweep`** (maturin, `bindings = "bin"`) while
installing the `sweep` binary. Releases are built and uploaded by
`.github/workflows/release.yml` on `v*` tags via PyPI trusted
publishing.

## License

[MIT](LICENSE)
