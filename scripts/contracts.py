import json
import os
import re
import subprocess
import sys
from pathlib import Path

RUSTC = re.compile(r"rustc \d+\.[^\s\"(),]*(?: \([0-9a-f]+ \d{4}-\d{2}-\d{2}\))?")
HELP_BLOCKS = ("Commands:", "Available subcommands:")
FIXED_ENV = {
    "PATH": "/usr/bin:/bin",
    "LANG": "C",
    "LC_ALL": "C",
}


def fixed_env(home):
    env = dict(FIXED_ENV)
    env["HOME"] = str(home)
    return env


def run(command, cwd=None, home=None):
    completed = subprocess.run(
        command,
        cwd=str(cwd) if cwd else None,
        env=fixed_env(home or Path.home()),
        capture_output=True,
        text=True,
    )
    return completed


def subcommand_names(help_text):
    names = []
    inside = False
    for line in help_text.splitlines():
        if line.strip() in HELP_BLOCKS:
            inside = True
            continue
        if not inside:
            continue
        if not line.strip() or not line.startswith("  "):
            inside = False
            continue
        names.append(line.strip().split()[0])
    return names


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    body = text.rstrip("\n") + "\n"
    path.write_text(body, encoding="utf-8")


def capture_cli(binary, out, prefix=(), home=None):
    file_name = "__".join(("poph",) + prefix) + ".txt"
    completed = run([binary] + list(prefix) + ["--help"], home=home)
    if completed.returncode != 0:
        raise SystemExit(
            "help failed for %s: %s" % (" ".join(prefix) or "poph", completed.stderr)
        )
    write(out / file_name, completed.stdout)
    for name in subcommand_names(completed.stdout):
        if name == "help":
            continue
        capture_cli(binary, out, prefix + (name,), home=home)


def canonical_json(value):
    return json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False)


def scrub(text, workdir):
    text = RUSTC.sub("rustc <version>", text)
    if workdir:
        text = text.replace(str(workdir), "<data_dir>")
    return text


def main(argv):
    command = argv[1]
    if command == "cli":
        out = Path(argv[2])
        binary = argv[3]
        home = Path(argv[4]) if len(argv) > 4 else None
        capture_cli(binary, out, home=home)
        version = run([binary, "--version"], home=home)
        write(out / "poph__version.txt", version.stdout)
        return 0
    if command == "canonical-json":
        source = Path(argv[2])
        target = Path(argv[3])
        value = json.loads(source.read_text(encoding="utf-8"))
        write(target, canonical_json(value))
        return 0
    if command == "doctor":
        out = Path(argv[2])
        binary = argv[3]
        workdir = Path(argv[4])
        workdir.mkdir(parents=True, exist_ok=True)
        steps = [
            ("doctor-empty", ["doctor", "--json"]),
            ("migrate-empty", ["migrate", "--json"]),
            ("migrate-again", ["migrate", "--json"]),
            ("doctor-migrated", ["doctor", "--json"]),
        ]
        for name, args in steps:
            completed = run([binary] + args, cwd=workdir, home=workdir)
            body = scrub(completed.stdout, workdir)
            if name.startswith("migrate"):
                (out / "db").mkdir(parents=True, exist_ok=True)
                write(out / "db" / (name + ".json"), body)
            else:
                write(out / "config" / (name + ".json"), body)
        return 0
    raise SystemExit("unknown command: %s" % command)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
