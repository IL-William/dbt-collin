#!/usr/bin/env python3
"""Ruff on the Python and ShellCheck on the shell (0056).

Each linter is the upstream GitHub release asset of one pinned version, checked
against the SHA-256 written here (GitHub's own digest of the asset, which for
Ruff is also the one in its .sha256 file) before anything of it is unpacked or
made executable, then cached under target/lint-tools/<tool>-<version>-<hash>/. So a
machine needs nothing installed, and CI and every checkout run the same
versions. Updating a linter means changing its version, its URLs, its hashes
and, for an archive, the path of the binary in it, together, and only to a
release at least seven days old, as for crates and actions: Dependabot cannot
see these pins.

The rules live in ruff.toml, each switched-off one with its reason. Nothing
here formats: no formatter is run over these languages.

One line per finding, at most 30 per tool and then a count; `<tool> ok` when it
found nothing. --github writes GitHub annotations instead, for CI.

Standard library only, Python 3.10 or later. Exit 0 when clean, 1 on a finding,
a linter that failed or a download whose hash is not the pinned one (deleted,
never run), 2 when a linter could not run here: offline with nothing cached, or
no pinned build for this platform. The last line then says which and why.
"""

import hashlib
import http.client
import os
import platform
import re
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / "target" / "lint-tools"
SHOWN = 30

# Per tool and per platform (sys.platform, machine): the asset URL, its SHA-256
# and the binary's path in the archive, None where the asset is the binary.
# Every pin here is an archive; the None path is kept in step with dbt-edith's
# copy of this script, whose Biome asset is a bare binary.
# Hashes are GitHub's digest of each release asset.
TOOLS = [
    {
        "name": "ruff",
        "version": "0.17.3",
        "assets": {
            ("darwin", "arm64"): (
                "https://github.com/astral-sh/ruff/releases/download/0.17.3/ruff-aarch64-apple-darwin.tar.gz",
                "118f7121aa838785672bf612487f755a89108bb5f44e0f3df96c5967a72614f9",
                "ruff-aarch64-apple-darwin/ruff",
            ),
            ("darwin", "x86_64"): (
                "https://github.com/astral-sh/ruff/releases/download/0.17.3/ruff-x86_64-apple-darwin.tar.gz",
                "17f95b1bee24d70de62ffe5beec7f71bb541c7e3d147a1c3d84f7cd6525f6fdf",
                "ruff-x86_64-apple-darwin/ruff",
            ),
            ("linux", "x86_64"): (
                "https://github.com/astral-sh/ruff/releases/download/0.17.3/ruff-x86_64-unknown-linux-gnu.tar.gz",
                "2b0890f5a55900af741f133918c2656919bbf18c2ab7741b4ca8403a90e647a9",
                "ruff-x86_64-unknown-linux-gnu/ruff",
            ),
        },
    },
    {
        "name": "shellcheck",
        "version": "0.11.0",
        "assets": {
            ("darwin", "arm64"): (
                "https://github.com/koalaman/shellcheck/releases/download/v0.11.0/shellcheck-v0.11.0.darwin.aarch64.tar.gz",
                "339b930feb1ea764467013cc1f72d09cd6b869ebf1013296ba9055ab2ffbd26f",
                "shellcheck-v0.11.0/shellcheck",
            ),
            ("darwin", "x86_64"): (
                "https://github.com/koalaman/shellcheck/releases/download/v0.11.0/shellcheck-v0.11.0.darwin.x86_64.tar.gz",
                "c2c15e08df0e8fbc374c335b230a7ee958c313fa5714817a59aa59f1aa594f51",
                "shellcheck-v0.11.0/shellcheck",
            ),
            ("linux", "x86_64"): (
                "https://github.com/koalaman/shellcheck/releases/download/v0.11.0/shellcheck-v0.11.0.linux.x86_64.tar.gz",
                "b7af85e41cc99489dcc21d66c6d5f3685138f06d34651e6d34b42ec6d54fe6f6",
                "shellcheck-v0.11.0/shellcheck",
            ),
        },
    },
]


# Never read: the lineage-library submodule (its own repository, its own
# checks) and the build folder of a fixture project.
SKIPPED = [
    ":(exclude)crates/collin-core/fixtures/lineage-library",
    ":(glob,exclude)crates/collin-core/fixtures/*/target/**",
]


class Refused(Exception):
    """A download whose SHA-256 is not the pinned one."""


class Unavailable(Exception):
    """The linter cannot be had here."""


class Broken(Exception):
    """A cached linter that will not start."""


def platform_key():
    machine = platform.machine().lower()
    return sys.platform, {"aarch64": "arm64", "amd64": "x86_64"}.get(machine, machine)


def binary(tool):
    """The linter's executable, downloaded and verified when it is not cached."""
    label = f"{tool['name']} {tool['version']}"
    key = platform_key()
    if key not in tool["assets"]:
        raise Unavailable(f"{label} has no pinned build for {key[0]}-{key[1]}")
    url, digest, member = tool["assets"][key]
    # The hash is in the folder's name, so a pin changed without its version
    # never finds the binary of the old one.
    folder = CACHE / f"{tool['name']}-{tool['version']}-{digest[:12]}"
    target = folder / tool["name"]
    if target.is_file() and os.access(target, os.X_OK):
        return target
    folder.mkdir(parents=True, exist_ok=True)

    # The download lands in a temporary file beside the cache, whole, and is
    # hashed before it is opened or made executable.
    sha = hashlib.sha256()
    fd, tmp_name = tempfile.mkstemp(dir=folder, prefix="download-")
    tmp = Path(tmp_name)
    try:
        try:
            with os.fdopen(fd, "wb") as out, urllib.request.urlopen(url, timeout=60) as res:
                while chunk := res.read(1 << 20):
                    sha.update(chunk)
                    out.write(chunk)
        except (OSError, http.client.HTTPException) as e:
            raise Unavailable(f"{label} is not cached and cannot be downloaded ({e})") from None
        if sha.hexdigest() != digest:
            raise Refused(f"{label}: the download's SHA-256 is {sha.hexdigest()}, the pinned one {digest}; refused and deleted")
        if member is None:
            data = tmp.read_bytes()
        else:
            # Only the one expected member is read, by name and never extracted
            # by its own path, so nothing in the archive can land elsewhere. An
            # archive of the pinned hash that lacks it is a wrong pin: refused.
            try:
                with tarfile.open(tmp, "r:gz") as tar:
                    info = tar.getmember(member)
                    if not info.isfile():
                        raise Refused(f"{label}: {member} in the archive is not a file")
                    data = tar.extractfile(info).read()
            except (KeyError, tarfile.TarError) as e:
                raise Refused(f"{label}: the archive has no usable {member} ({e})") from None
        # Written whole under a name of its own, made executable, then renamed:
        # a half-written binary is never the cached one.
        sfd, staged_name = tempfile.mkstemp(dir=folder, prefix="staged-")
        staged = Path(staged_name)
        try:
            with os.fdopen(sfd, "wb") as out:
                out.write(data)
            staged.chmod(0o755)
            os.replace(staged, target)
        finally:
            staged.unlink(missing_ok=True)
    finally:
        tmp.unlink(missing_ok=True)
    return target


def listed(*patterns):
    """The files git knows or would add that match, so a build folder or a vendored
    copy is never read, a new file is read before it is added, and one deleted
    without `git rm` is left out."""
    command = ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", *patterns, *SKIPPED]
    try:
        out = subprocess.run(command, cwd=ROOT, capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as e:
        raise Unavailable(f"cannot list the files ({e})") from None
    return sorted({f for f in out.decode().split("\0") if f and (ROOT / f).is_file()})


def run(command):
    try:
        done = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=False)
    except OSError as e:
        # A binary of another OS in a target/ shared between them, for one.
        raise Broken(f"cannot start {command[0]} ({e}); delete target/lint-tools to fetch it again") from None
    return done.returncode, done.stdout, done.stderr


def ruff(exe, github):
    files = listed("*.py")
    if not files:
        return [], 0, ""
    fmt = "github" if github else "concise"
    code, out, err = run([str(exe), "check", "--no-cache", "--output-format", fmt, "--config", "ruff.toml", *files])
    pattern = r"::" if github else r"\S+:\d+:\d+: "
    return [x for x in out.splitlines() if re.match(pattern, x)], code, err


def shellcheck(exe, github):
    files = listed("*.sh")
    if not files:
        return [], 0, ""
    code, out, err = run([str(exe), "--format", "gcc", *files])
    found = []
    for raw in out.splitlines():
        m = re.match(r"(.+?):(\d+):(\d+): (\w+): (.*)", raw)
        if not m:
            continue
        if github:
            path, row, col, level, text = m.groups()
            kind = {"error": "error", "warning": "warning"}.get(level, "notice")
            found.append(f"::{kind} file={path},line={row},col={col}::{text}")
        else:
            found.append(raw)
    return found, code, err


CHECKS = {"ruff": ruff, "shellcheck": shellcheck}


def main(argv):
    if any(a not in ("--github",) for a in argv):
        print(__doc__.split("\n\n")[0] + "\nusage: lint.py [--github]", file=sys.stderr)
        return 2
    github = "--github" in argv
    refused = failed = False
    skipped = []
    for tool in TOOLS:
        try:
            exe = binary(tool)
            found, code, err = CHECKS[tool["name"]](exe, github)
        except Refused as e:
            print(f"REFUSED {e}")
            refused = True
            continue
        except Unavailable as e:
            skipped.append(str(e))
            continue
        except Broken as e:
            print(f"{tool['name']}: {e}")
            failed = True
            continue
        if found:
            failed = True
            for line in found[:SHOWN]:
                print(line)
            print(f"{tool['name']}: {len(found)} finding(s)" + (f", the first {SHOWN} shown" if len(found) > SHOWN else ""))
        elif code != 0:
            # No finding it could read, yet it failed: a broken config or a crash.
            failed = True
            detail = " ".join(err.strip().splitlines()[:1])
            print(f"{tool['name']}: failed with exit {code}, no finding read. {detail}".rstrip())
        else:
            print(f"{tool['name']} ok")
    if skipped:
        print("SKIPPED: " + "; ".join(skipped))
    if refused or failed:
        return 1
    return 2 if skipped else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
