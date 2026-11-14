# VGI Symbols notebooks

[01 · Symbol availability and inline expansion are different dimensions](01_symbol_coverage_and_inline_expansion.ipynb)
uses visible Haybarn SQL to inspect debug modules, configure local sources,
symbolicate frame columns and expand inline chains while preserving every input.
Two charts separate resolution coverage from expanded row count. Saved outputs are
from the real published worker, using authored frames and committed debug fixtures.

## Run

Install `uv`, then from this directory:

```sh
uv sync --locked
uv run --locked python execute.py
```

The entire `notebooks/` directory works when copied elsewhere; no worker checkout,
compiler, live process or external symbol server is needed. Internet access is
required initially for dependencies, the community VGI extension and native release.
Your platform needs a matching release asset. Tested on macOS Apple silicon with
Haybarn 2.0.0rc11. The debug fixtures are parsed as data and never executed.

The runner uses a fresh temporary kernel and this directory as its working directory,
replacing saved outputs only after successful execution. For VS Code, select this
directory's `.venv/bin/python`, or register it:

```sh
uv run --locked python -m ipykernel install --user --name vgi-symbols-notebooks --display-name 'Python (vgi-symbols notebooks)'
```

**Restart the kernel before each full run or parameter experiment.** Worker sources,
resident modules and negative-cache entries can survive a new connection in the
same kernel. A pre-configuration guard requires empty source/cache state and asks
for a restart instead of clearing unrelated state. Keep this directory as the
interactive working directory. Connection and analytical SQL stay visible; helper
code and cleanup are folded.

## Experiment

```sh
uv run --locked python execute.py --parameters '{"INCLUDE_COMPUTE_SYMBOLS": true}' --output-dir validation/compute
```

Overrides require a separate destination, which receives branding, fixtures and
environment companions. Enabling the second local debug source changes two compute
frames from `not_found` to `ok`: file/line coverage rises from four to six of ten
inputs. Both modes preserve ten input records and sixteen expanded rows, including
six inline rows. The expansion chart stays unchanged. Reset `False` for the saved
example, with a fresh kernel.

## Worker and fixture provenance

The literal native location is
`github-auto://Query-farm/vgi-symbols@v0.4.0`. It downloads a native release, checks its
hash and caches it. To upgrade, change the visible pin and verify both experiments.
Notebook dependencies use their own locked environment. The worker's license is
documented in the repository's root `LICENSE`.

`macho_inline.dwarf` and `macho_compute.dwarf` are unchanged files from `test/symbols`.
`prog.c` and `prog2.c` are unchanged provenance sources from
`crates/symbols-core/tests/fixtures`. Their source revision and all five fixture
hashes are recorded in `fixtures/manifest.json`. `frames.json` contains ten authored
records using golden-test virtual addresses, repeated addresses, an unknown module,
a far-out address and missing arguments. This is not a real crash or profiling sample.

```sh
uv run --locked python fixtures/build_frames.py
```

That command regenerates only frame JSON and the manifest; it does not compile or
modify the bundled debug modules. Rebuilding debug fixtures can change IDs and
addresses, so the pinned files are supplied intact. Debug information contains original
build-source paths; displayed file names are reduced to basenames. Those paths are
metadata, not local files the notebook reads.

Addresses are the fixture's preferred virtual addresses before any runtime slide.
This notebook does not calculate ASLR or load-map corrections. The standalone debug
files report `image_base=0`; that is not evidence of a real process's load bias.
Both modules can be inspected even when one source is disabled; inspection does not
register a resolution source. Configured globs point only at bundled files.

## Interpretation and validation

The scalar's function is the physical function; its inline list excludes that
physical frame. `function_name` instead returns the innermost inline name when one
exists. Expansion explicitly retains one physical-record row per input, including
unknown and missing cases. It adds inline rows without turning them into new samples.
The no-line fixture has a fallback name, so name presence alone is not exact source
coverage. Missing arguments return `not_found` in this release; the input audit keeps
them separate from an unknown module. Cache counters are not a timing benchmark or
an inferred parse count.

Default and detached experiment runs pass twelve SQL checks and two chart
reconciliations. Both charts are inspected in both modes. Three runner guards cover
overrides without a destination, unknown parameters, and failed invalid-parameter
execution preserving outputs. Authored frames regenerate identically; all copied
fixture bytes match their source and manifest hashes.

Repository validation passes locked release build, 31 Rust tests, Clippy, formatting,
warning-clean Rust documentation, 55 SQL assertions in three cases, cargo audit and
the Rust 1.97 workspace check. All three CI fuzz targets pass their 45-second budgets
(about 8.5 million runs combined). Metadata scores 100/100, L2, zero findings with
structural and behavioral verification. Source and root dependencies remain unchanged.

Official Query.Farm Strata Sun and VGI assets are bundled unchanged. The wordmark
links to Query.Farm at the top and final bottom of the notebook.
