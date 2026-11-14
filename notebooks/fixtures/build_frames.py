"""Write authored frame records keyed to the unchanged bundled debug fixtures."""

from pathlib import Path
import hashlib
import json


def main() -> None:
    """Regenerate frame JSON and hash all bundled provenance companions."""
    root = Path(__file__).resolve().parent
    inline = "76ff8518da153e64a8403892fcbf11250"
    compute = "4930eb84c7b63ef6a97fbdcdd473f5a30"
    values = [
        (1, inline, 4294968116),
        (2, inline, 4294968116),
        (3, inline, 4294968120),
        (4, inline, 4294968104),
        (5, inline, 18446744073709486080),
        (6, compute, 4294968104),
        (7, compute, 4294968112),
        (8, "00000000000000000000000000000000a", 4294968116),
        (9, None, 4294968116),
        (10, inline, None),
    ]
    (root / "frames.json").write_text(
        json.dumps([dict(frame_id=i, build_id=b, address=a) for i, b, a in values], indent=2) + "\n"
    )
    names = ["frames.json", "macho_inline.dwarf", "macho_compute.dwarf", "prog.c", "prog2.c"]
    (root / "manifest.json").write_text(
        json.dumps(
            {
                "source_commit": "1d76daf3589edc489960191781888f1020cd72d9",
                "note": "Authored frame records use golden-test virtual addresses. Debug modules and C provenance sources are unchanged repository fixtures; binaries are parsed, never executed.",
                "sha256": {
                    name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in names
                },
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
