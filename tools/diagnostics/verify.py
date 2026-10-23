"""Compare every frame of interpreter, single-library and multi-library runs."""
import argparse
import pathlib
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=pathlib.Path, required=True)
    parser.add_argument("--rom", type=pathlib.Path, required=True)
    parser.add_argument("--input", type=pathlib.Path, required=True)
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--library", type=pathlib.Path, action="append", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--audio-hashes", action="store_true")
    parser.add_argument("--require-native", action="store_true", help="Require zero interpreted instructions in multi mode")
    args = parser.parse_args()
    if len(args.library) < 2 or args.frames < 1:
        parser.error("Need at least two libraries and a positive frame count")
    args.output.mkdir(parents=True, exist_ok=True)
    baseline = None
    for mode, libs in [("interp", []), ("single", args.library[:1]), ("multi", args.library)]:
        csv = args.output / f"{mode}.csv"
        command = [str(args.probe.resolve()), str(args.rom.resolve()),
                   str(args.input.resolve()), str(args.frames), str(csv.resolve()),
                   mode, *(str(p.resolve()) for p in libs)]
        if args.audio_hashes:
            command.append("--audio-hashes")
        if args.require_native and mode == "multi":
            command += ["--require-native", "--audit", str((args.output / "fallbacks.csv").resolve())]
        result = subprocess.run(command, text=True, capture_output=True)
        (args.output / f"{mode}.log").write_text(result.stdout + result.stderr, encoding="utf-8")
        print(result.stdout, end="", flush=True)
        if result.returncode:
            raise RuntimeError(f"{mode} failed: {result.stderr}")
        rows = csv.read_text().splitlines()
        if len(rows) != args.frames + 1:
            raise RuntimeError(f"{mode}: unexpected frame count")
        if baseline is None:
            baseline = rows
        elif rows != baseline:
            first = next(i for i, pair in enumerate(zip(baseline, rows)) if pair[0] != pair[1])
            raise RuntimeError(f"{mode}: mismatch at CSV row {first}: {baseline[first]} != {rows[first]}")
    print(f"MATCH: all {args.frames} frame hashes in all three modes", flush=True)


if __name__ == "__main__":
    main()
