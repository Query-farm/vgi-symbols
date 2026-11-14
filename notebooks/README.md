# Native symbolication walkthrough

[01 · Symbol availability and inline expansion are different dimensions](01_symbol_coverage_and_inline_expansion.ipynb)
uses visible Haybarn SQL to inspect debug modules, choose and audit local symbol
sources, resolve a frame column, compare physical and innermost function names,
and expand inline chains while retaining every input record.

Ten **authored** frames use unchanged committed Mach-O debug fixtures. Duplicate
addresses remain separate frame records. No live process is sampled and no fixture
binary is executed. Counts describe these inputs, not CPU time, crash frequency or
incident counts. Two charts separate file/line resolution coverage from inline-expanded
row count.

## Run locally or in Colab

Install [uv](https://docs.astral.sh/uv/), then from `notebooks/`:

```sh
uv sync --locked
uv run --locked vgi-notebooks 01_symbol_coverage_and_inline_expansion.ipynb
```

For an editor, select `notebooks/.venv/bin/python` and use `notebooks/` as the working
directory. The Colab badge opens the same source, and a notebook-only copy can fetch
its immutable fixtures through the shared registry. The visible bootstrap installs
only shared base dependencies; local publication adds `[runner]`. Shared **v0.3.22**
and Haybarn **2.0.0rc11** are pinned separately from the native worker. SQL starts with
an empty temporary secret directory and persistent secrets disabled.

Initial setup needs network access for Python dependencies, the community VGI
extension, fixture companions and a matching native release asset. There is no
compiler, worker checkout, live process or external symbol server requirement. The
release supplies Linux x86_64/arm64, macOS arm64 and Windows x86_64 artifacts; actual
execution here used macOS arm64. Hosted Colab was not manually run; a fresh preloaded
Python 3.12 kernel verifies full notebook-only bootstrap and execution.

The visible connection is:

```sql
ATTACH 'symbols' (TYPE vgi,
 LOCATION 'github-auto://Query-farm/vgi-symbols@v0.4.0');
```

The native transport downloads the published release, verifies its checksum and caches
it. To upgrade, change the visible release and recheck both parameter modes. The
worker's source-available license is documented in the root [LICENSE](../LICENSE).

**Restart the kernel before each complete run or experiment.** Configured sources,
resident modules and negative-cache entries can survive a new connection within the
same kernel. A pre-configuration guard requires empty source/cache state and asks for
a restart instead of clearing unrelated state. Only local exact-file globs are enabled;
no remote lookup is configured.

## Fixture provenance and address semantics

The shared `symbols` registry retrieves seven unchanged files from notebook commit
`5e610ee67f4c52f9950cfd65216071deb962ac08`. `prepare("symbols")` verifies their hashes
and establishes the fixture-relative working directory. The notebook additionally
checks [fixtures/manifest.json](fixtures/manifest.json) before connecting.

`macho_inline.dwarf` and `macho_compute.dwarf` are unchanged repository debug files;
`prog.c` and `prog2.c` are their unchanged provenance sources. The manifest records
source revision `1d76daf3589edc489960191781888f1020cd72d9` and all five data/source
hashes. `frames.json` contains golden-test preferred virtual addresses, repeated
addresses, an unknown module, a far-out address and missing arguments. It is not a
real crash or profiling sample.

```sh
uv run --locked python fixtures/build_frames.py
```

This optional command regenerates only the authored frame JSON and manifest. It
neither compiles nor modifies the supplied debug modules. Rebuilding debug fixtures
can change IDs and addresses, so their original bytes are supplied intact. Debug
metadata includes original build-source paths; displayed filenames are reduced to
basenames. Those metadata paths are not files the notebook opens.

Addresses are fixture virtual addresses before any runtime slide. This notebook does
not perform ASLR or load-map correction. Standalone debug files report `image_base=0`;
that is not a process load-bias measurement and does not justify rewriting the known
fixture addresses. `module_info` can inspect both modules even when one source is
disabled; inspection does not register a resolution source.

## Focused experiment

Change only availability of the compute module, preserving the successful default:

```sh
uv run --locked vgi-notebooks 01_symbol_coverage_and_inline_expansion.ipynb \
  --parameters '{"INCLUDE_COMPUTE_SYMBOLS":true}' \
  --output-dir validation/compute
```

Predict that frames 6 and 7 change from `not_found` to `ok`. File/line coverage rises
from **four to six of ten** input frames. Both modes preserve ten input records and
sixteen expanded rows, including six inline rows. The compute addresses have no inline
chain, so the expansion chart stays identical. Compare the physical/innermost-name
table too; the fixed inline fixture keeps the same naming distinction. Reset `False`
with a fresh kernel for the saved default.

The runner requires a separate destination for overrides, rejects unknown parameters,
and replaces saved outputs only after every cell succeeds. Published charts are linked
SVGs under `charts/`; ignored PNG previews and logs are under `artifacts/` and
`validation/`. Detached experiment copies carry their required companions.

## Interpretation and verified coverage

`symbolicate` supplies a physical function and a list of inline callees, innermost
first. `function_name` returns the innermost name when present: frames 1 and 2 retain
physical `apply` but have innermost `inner_lo`. A grouping key should name which grain
it represents. This separate shortcut query is another resolution pass; cache counters
must not be read as exactly one request per authored input or as a parse/timing benchmark.

Expansion retains one physical-record row per input, including unknown and missing
cases. Inline rows do not become new samples. The no-line fixture has a fallback name,
so name presence alone is not exact source coverage. Missing arguments return
`not_found` in this release; the input audit distinguishes them from an unknown module.

[coverage.json](coverage.json) maps **seven of eleven** released and current public
names to executed visible SQL, meaningful results and editorial findings. Complete
native function/table/view exports live under [coverage/](coverage/), including the
`source_kinds` view and empty table export. Overloads count once; mutation capabilities
remain in the denominator. This focused story leaves `demangle`, `inline_frames`,
`cache_evict` and `drop_source` uncovered. Broad design docs mention additional table
forms and a version helper that these actual runtime catalogs do not expose; they are
not silently inferred into or removed from the collected catalog.

Both the actual public **v0.4.0** release and current source baseline
`5e610ee67f4c52f9950cfd65216071deb962ac08` expose those eleven names. Current metadata
adds composed examples; core resolution and scalar implementations are unchanged.
The existing current binary was reused for catalog collection without rebuilding.
Final notebook receipts describe release execution; current-interface compatibility is
reviewed separately and is not an exact-current notebook execution claim.

Validation includes the saved default, a detached notebook-only compute-source
experiment, thirteen SQL invariants, two chart reconciliations, three runner guards
and two invalid-parameter guards. Fixture hashes and regeneration agree. A fresh
Python 3.12 kernel preserves preloaded pandas/Matplotlib/IPython/ipykernel/jupyter-client
versions while running the base-only bootstrap and complete notebook. Four charts are
reviewed across the two modes.

For this notebook-only change, existing green [source CI](https://github.com/Query-farm/vgi-symbols/actions/runs/43564601307)
is reused. That source revision differs from the publication baseline only by the
previous notebook addition and README link. No worker rebuild, source test repetition,
root dependency change or original-checkout modification is required.

An additional detached attempt with an empty native-release cache received HTTP 403
from GitHub's release-metadata endpoint. The successful parameter experiment reused
the already verified native cache; its fresh fixture/bootstrap checks are separate.
The initial HTTP failure is retained in validation evidence, and no alternate worker
or fabricated output was substituted. A first-time reader still needs GitHub's release
endpoint and matching artifact to be available.
