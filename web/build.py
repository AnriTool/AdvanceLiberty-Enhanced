"""Build the static web package. Run from any working directory."""

from pathlib import Path
import json
import shutil
import subprocess


def main():
    web = Path(__file__).resolve().parent
    root = web.parent
    target = root / "target"
    subprocess.run(
        [
            "cargo", "build", "--locked", "--release",
            "--package", "advance-liberty-web",
            "--target", "wasm32-unknown-unknown", "--target-dir", str(target),
        ],
        cwd=root,
        check=True,
    )
    output = root / "dist" / "web"
    output.mkdir(parents=True, exist_ok=True)
    for name in ("index.html", "style.css", "play.js", "input.mjs"):
        shutil.copyfile(web / name, output / name)
    shutil.copyfile(
        target / "wasm32-unknown-unknown" / "release" / "advance_liberty_web.wasm",
        output / "advance_liberty_web.wasm",
    )
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(root / name, output / name)
    licenses = output / "licenses"
    licenses.mkdir(exist_ok=True)
    shutil.copyfile(root / "release" / "THIRD_PARTY_NOTICES.txt", licenses / "THIRD_PARTY_NOTICES.txt")
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copyfile(root / "vendor" / name, licenses / f"gba-recomp-{name}.txt")
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1",
         "--filter-platform", "wasm32-unknown-unknown"],
        cwd=root,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "advance-liberty-web")
    graph = {node["id"]: node["dependencies"] for node in metadata["resolve"]["nodes"]}
    pending = [package["id"]]
    reachable = set()
    while pending:
        package_id = pending.pop()
        if package_id not in reachable:
            reachable.add(package_id)
            pending.extend(graph[package_id])
    dependencies = []
    for package in metadata["packages"]:
        if package["id"] not in reachable or package["source"] is None:
            continue
        dependencies.append({key: package[key] for key in ("name", "version", "license")})
        destination = licenses / f'{package["name"]}-{package["version"]}'
        for file in Path(package["manifest_path"]).parent.iterdir():
            if file.is_file() and file.name.upper().startswith(
                ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE")
            ):
                destination.mkdir(exist_ok=True)
                shutil.copyfile(file, destination / file.name)
    (licenses / "dependencies.json").write_text(json.dumps(dependencies, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
