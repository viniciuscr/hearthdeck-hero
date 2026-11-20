#!/usr/bin/env python3
"""Works out the fork's next release and writes the notes it publishes.

A fork release is tagged `v<upstream>-hearthdeck.<counter>`, for example
`v1.7.0-hearthdeck.1.5.0`: `1.7.0` is the upstream version the tree is built
from (the `[workspace.package] version` in Cargo.toml, which the sync brings
along with upstream's own commits) and `1.5.0` counts the fork's releases.
The counter is a version of its own: a `feat` bumps the minor, a `fix` (or
anything else) the patch, and a `!` / `BREAKING CHANGE` the major.

    scripts/fork/release.py                 # what the next release would say
    scripts/fork/release.py --tag           # its tag name          (the workflow)
    scripts/fork/release.py --notes         # its notes, markdown   (the workflow)
    scripts/fork/release.py --bump minor    # override the derived bump
    scripts/fork/release.py --publish       # tag HEAD with the notes and push it
    scripts/fork/release.py --check TAG     # validate a tag before publishing

The notes have two halves. "What we changed" is the fork's own commits, taken
`--first-parent` so the upstream side of every sync merge is left out. "From
upstream" is everything else the window brought in, summarised: the top
scopes, a count of the rest, and a link to upstream's own release notes --
a release note that lists sixty upstream commits buries the fork's work.
Only user-facing Conventional Commit types are listed; documentation, chores,
CI and tests are dropped, as `cliff.toml` drops them.

`--first-parent` is the split because the integration branch is linear: feature
branches are squash-merged and only an upstream sync merges (docs/FORK.md), so
a `--no-ff` merge of a feature would read as an upstream change. Keep releases
on a linear fork/main.

The notes go into the annotated tag, which is what the release workflow
publishes (`git tag -l --format='%(contents:body)'`), so a tag is a complete
record of what it shipped. No third-party packages. See docs/RELEASING.md.
"""

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# Upstream's release page tells the whole story of an upstream change; the
# fork's notes name the highlights and link there.
UPSTREAM_REPO = "https://github.com/RafaelGoulartB/Heronite"

UPSTREAM_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
FORK_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)-hearthdeck\.(\d+)(?:\.(\d+))?(?:\.(\d+))?$")
CONVENTIONAL = re.compile(r"^([a-z]+)(?:\(([^)]+)\))?(!)?: (.+)$")
# `[workspace.package]` and its version, as scripts/bump-version.py reads them.
SECTION = re.compile(r"^\[workspace\.package\]\n(?:(?!^\[).*\n)*", re.M)
VERSION = re.compile(r'^version = "(\d+)\.(\d+)\.(\d+)"$', re.M)

# The user-facing types, in the order they are listed (cliff.toml's groups).
# Everything else — chore, ci, build, docs, style, refactor, test, deps, merge
# — is bookkeeping: it says nothing a person running the app sees, so a commit
# that is not one of these types is not part of the notes.
GROUPS = (
    ("feat", "✨ Features"),
    ("fix", "🐛 Bug fixes"),
    ("perf", "⚡ Performance"),
    ("ui", "🎨 Interface"),
    ("revert", "⏪ Reverted"),
)
LISTED = frozenset(kind for kind, _ in GROUPS)
# The upstream half names this many scopes, this many subjects from each.
HIGHLIGHTED = 5
PER_SCOPE = 2
# Where the baseline's own ref name lives: a checkout may have either.
BASELINE_REFS = ("main", "origin/main")


class Fault(Exception):
    """What the person must fix; reported as one line and exit status 2."""


def root() -> Path:
    done = subprocess.run(["git", "rev-parse", "--show-toplevel"], text=True, capture_output=True)
    if done.returncode or not done.stdout.strip():
        raise Fault("not inside a git work tree")
    return Path(done.stdout.strip())


ROOT = root()


def git(*args: str, check: bool = True) -> str | None:
    """Runs git in the work tree; `check=False` returns None when it fails."""
    done = subprocess.run(["git", *args], cwd=ROOT, text=True, capture_output=True)
    if done.returncode:
        if not check:
            return None
        raise Fault(f"git {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout


def is_ancestor(commit: str, of: str) -> bool:
    done = subprocess.run(
        ["git", "merge-base", "--is-ancestor", commit, of], cwd=ROOT, capture_output=True
    )
    if done.returncode not in (0, 1):
        raise Fault(f"git merge-base --is-ancestor {commit} {of}: {done.stderr.decode().strip()}")
    return done.returncode == 0


def exists(rev: str) -> bool:
    return git("rev-parse", "--verify", "--quiet", rev, check=False) is not None


def join(version: tuple[int, ...]) -> str:
    return ".".join(str(part) for part in version)


def workspace_version(rev: str) -> tuple[int, int, int]:
    """`[workspace.package] version` at `rev`: the upstream base of the tree."""
    text = git("show", f"{rev}:Cargo.toml") or ""
    section = SECTION.search(text)
    found = section and VERSION.search(section.group(0))
    if not found:
        raise Fault(f'no version = "X.Y.Z" in [workspace.package] at {rev}')
    return tuple(int(part) for part in found.groups())


def releases(rev: str | None = "HEAD") -> list[dict]:
    """Every fork release tag reachable from `rev`, oldest first.

    A checkout of an old commit must not see the releases made after it, and
    a tag from a branch that never merged is not part of this line at all.
    `rev=None` means every tag: `--check` must still see a newer release.
    """
    found = []
    for name in (git("tag", "--list") or "").splitlines():
        match = FORK_TAG.match(name)
        if not match or (rev is not None and not is_ancestor(name, rev)):
            continue
        found.append(
            {
                "tag": name,
                "base": tuple(int(part) for part in match.group(1, 2, 3)),
                "counter": tuple(int(part or 0) for part in match.group(4, 5, 6)),
            }
        )
    return sorted(found, key=lambda release: (release["base"], release["counter"]))


def baseline() -> str | None:
    for ref in BASELINE_REFS:
        if exists(ref):
            return ref
    return None


def commits(span: str, first_parent: bool = False) -> list[dict]:
    """The conventional commits in `span`, oldest first."""
    field, record = "\x1f", "\x1e"
    args = ["log", "--no-merges", f"--format=%h{field}%s{field}%b{record}"]
    if first_parent:
        args.append("--first-parent")
    args.append(span)
    found = []
    for chunk in (git(*args) or "").split(record):
        chunk = chunk.strip("\n")
        if not chunk:
            continue
        sha, subject, body = (chunk.split(field) + ["", ""])[:3]
        match = CONVENTIONAL.match(subject)
        if not match:
            continue
        kind, scope, bang, text = match.groups()
        if kind not in LISTED:
            continue
        found.append(
            {
                "sha": sha,
                "kind": kind,
                "scope": scope,
                "text": text.strip(),
                "breaking": bool(bang) or "BREAKING CHANGE" in body.upper(),
            }
        )
    return list(reversed(found))


def unduplicated(items: list[dict]) -> list[dict]:
    """Drops repeats: a conflict-resolved commit can land twice with one text."""
    seen, kept = set(), []
    for item in items:
        key = (item["kind"], item["scope"], item["text"])
        if key in seen:
            continue
        seen.add(key)
        kept.append(item)
    return kept


def bump_for(items: list[dict]) -> str:
    if any(item["breaking"] for item in items):
        return "major"
    if any(item["kind"] == "feat" for item in items):
        return "minor"
    return "patch"


def bumped(counter: tuple[int, int, int], bump: str) -> tuple[int, int, int]:
    major, minor, patch = counter
    if bump == "major":
        return (major + 1, 0, 0)
    if bump == "minor":
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)


def upstream_tags_between(previous: str | None, head: str) -> list[str]:
    """Upstream's `vX.Y.Z` releases that came in since the last fork release."""
    found = []
    for name in (git("tag", "--list") or "").splitlines():
        if not UPSTREAM_TAG.match(name) or not is_ancestor(name, head):
            continue
        if previous is not None and is_ancestor(name, previous):
            continue
        found.append(name)
    return sorted(found, key=lambda name: tuple(int(p) for p in UPSTREAM_TAG.match(name).groups()))


def newest_unmerged_upstream(base: tuple[int, int, int]) -> str | None:
    """Upstream's newest release tag the tree does not build on yet."""
    mirror = baseline()
    if mirror is None:
        return None
    newest = None
    for name in (git("tag", "--list") or "").splitlines():
        match = UPSTREAM_TAG.match(name)
        if not match or not is_ancestor(name, mirror):
            continue
        version = tuple(int(part) for part in match.groups())
        if version > base and (newest is None or version > newest[0]):
            newest = (version, name)
    return newest[1] if newest else None


def split_counts(span: str) -> tuple[int, int]:
    """How many non-merge commits the window holds on each side of the sync."""
    everyone = set((git("rev-list", "--no-merges", span) or "").split())
    own = set((git("rev-list", "--no-merges", "--first-parent", span) or "").split())
    return len(own), len(everyone - own)


def plan(bump: str) -> dict:
    """The next release: its tag, its commits, its upstream window."""
    base = workspace_version("HEAD")
    history = releases()
    last = history[-1] if history else None
    span = f"{last['tag']}..HEAD" if last else "HEAD"
    # The counts decide whether there is anything to release at all, so they
    # count every commit: a window of documentation and chores still ships.
    own_total, upstream_total = split_counts(span)
    ours = unduplicated(commits(span, first_parent=True))
    theirs = unduplicated([c for c in commits(span) if c["sha"] not in {o["sha"] for o in ours}])
    moved = last is None or base != last["base"]
    if own_total == 0 and upstream_total == 0 and not moved:
        raise Fault(f"nothing to release: no commit since {last['tag']}")
    if bump == "auto":
        bump = bump_for(ours)
    counter = bumped(last["counter"], bump) if last else (1, 0, 0)
    tag = f"v{join(base)}-hearthdeck.{join(counter)}"
    if exists(f"refs/tags/{tag}"):
        raise Fault(f"{tag} already exists; pass --bump to go further")
    mirror = baseline()
    compare = None
    if last is not None and mirror is not None:
        before = (git("merge-base", last["tag"], mirror) or "").strip()
        after = (git("merge-base", "HEAD", mirror) or "").strip()
        if before and after and before != after:
            compare = (before, after)
    return {
        "tag": tag,
        "base": base,
        "last": last,
        "ours": ours,
        "theirs": theirs,
        "bump": bump,
        "counter": counter,
        # The first release has no previous upstream base to compare against,
        # so naming "every upstream release since the fork started" would be
        # true and useless.
        "crossed": upstream_tags_between(last["tag"], "HEAD") if last else [],
        "compare": compare,
        "upstream_total": upstream_total,
    }


def sentence(text: str) -> str:
    """A commit's subject as a bullet: prose gets its first letter capital."""
    text = text.strip()
    if not text or not text[0].islower():
        return text
    head = text.split(" ", 1)[0]
    # Identifiers (`i18n`, `dbus_name`, `a/b`) keep the case they were written in.
    if any(char in head for char in "_/:") or any(char.isdigit() for char in head):
        return text
    return text[0].upper() + text[1:]


def fork_lines(release: dict) -> list[str]:
    lines = ["## What we changed", ""]
    if not release["ours"]:
        return lines + ["Nothing on the fork side: this release is upstream's.", ""]
    for kind, title in GROUPS:
        group = [item for item in release["ours"] if item["kind"] == kind]
        if not group:
            continue
        scopes = {item["scope"] for item in group}
        # One area for the whole group: name it once instead of once per line,
        # which is what a Watch-heavy release would otherwise repeat.
        one = len(scopes) == 1 and None not in scopes
        lines.append(f"### {title} ({next(iter(scopes))})" if one else f"### {title}")
        for item in group:
            scope = f"**{item['scope']}:** " if item["scope"] and not one else ""
            lines.append(f"- {scope}{sentence(item['text'])}")
        lines.append("")
    return lines


def tally(theirs: list[dict]) -> str | None:
    """The upstream changes that are not named individually: one sentence."""
    parts = []
    fixes = [item for item in theirs if item["kind"] == "fix"]
    if fixes:
        counted: dict[str, int] = {}
        for item in fixes:
            if item["scope"]:
                counted[item["scope"]] = counted.get(item["scope"], 0) + 1
        main = sorted(counted, key=lambda scope: (-counted[scope], scope))[:3]
        where = f" (mostly in {', '.join(main)})" if main else ""
        parts.append(f"{len(fixes)} fix{'' if len(fixes) == 1 else 'es'}{where}")
    for kind, one, many in (
        ("perf", "performance change", "performance changes"),
        ("ui", "interface change", "interface changes"),
        ("revert", "revert", "reverts"),
    ):
        total = sum(1 for item in theirs if item["kind"] == kind)
        if total:
            parts.append(f"{total} {one if total == 1 else many}")
    if not parts:
        return None
    joined = parts[0] if len(parts) == 1 else f"{', '.join(parts[:-1])} and {parts[-1]}"
    return f"Also {joined}."


def upstream_lines(release: dict) -> list[str]:
    lines = ["## From upstream", ""]
    theirs = release["theirs"]
    # Every commit, not just the listed ones: a sync that only brought
    # upstream's release chore still moved the tree.
    if not release["upstream_total"]:
        return lines + ["Upstream did not move since the last release.", ""]
    if release["crossed"]:
        named = ", ".join(f"[{tag}]({UPSTREAM_REPO}/releases/tag/{tag})" for tag in release["crossed"])
        lines += [f"Upstream released {named}.", ""]
    else:
        lines += ["Upstream's `main` advanced since the last release.", ""]
    if not theirs:
        return lines + ["No user-facing upstream changes: housekeeping only.", ""]
    features = [item for item in theirs if item["kind"] == "feat"]
    if features:
        lines.append("### ✨ Features")
        per_scope: dict[str, list[dict]] = {}
        for item in features:
            per_scope.setdefault(item["scope"] or "", []).append(item)
        named = sorted(per_scope, key=lambda scope: (-len(per_scope[scope]), scope))[:HIGHLIGHTED]
        shown = 0
        for scope in named:
            picked = per_scope[scope][:PER_SCOPE]
            shown += len(picked)
            text = "; ".join(sentence(item["text"]) for item in picked)
            lines.append(f"- **{scope}:** {text}" if scope else f"- {text}")
        if len(features) > shown:
            left = len(features) - shown
            rest = sorted(scope or "general" for scope in per_scope if scope not in named)
            where = f" ({', '.join(rest[:4])})" if rest else ""
            lines.append(f"- {left} more feature{'' if left == 1 else 's'}{where}")
        lines.append("")
    rest = tally(theirs)
    if rest:
        lines += [rest, ""]
    if release["compare"]:
        before, after = release["compare"]
        link = f"[{before[:7]}...{after[:7]}]({UPSTREAM_REPO}/compare/{before}...{after})"
        lines.append(f"Full upstream changes: {link}")
    else:
        lines.append(f"Upstream releases: {UPSTREAM_REPO}/releases")
    lines.append("")
    return lines


def repo_url() -> str | None:
    """The fork's web address, from origin (a token may sit in its URL)."""
    done = subprocess.run(["git", "remote", "get-url", "origin"], cwd=ROOT, text=True, capture_output=True)
    if done.returncode:
        return None
    match = re.search(r"github\.com[:/]([^/\s]+/[^/\s]+?)(?:\.git)?$", done.stdout.strip())
    return f"https://github.com/{match.group(1)}" if match else None


def render(release: dict) -> str:
    """The release notes: the tag message, and what GitHub publishes."""
    lines = [f"Hearthdeck Hero {release['tag']}", ""]
    if not release["ours"] and not release["upstream_total"]:
        lines += [
            "No user-facing changes: this release is housekeeping (documentation,",
            "tests, CI and chores only).",
            "",
        ]
    else:
        lines += fork_lines(release)
        lines += upstream_lines(release)
    origin = repo_url()
    if origin and release["last"]:
        previous, tag = release["last"]["tag"], release["tag"]
        lines += [f"Full changelog: [{previous}...{tag}]({origin}/compare/{previous}...{tag})"]
    return "\n".join(lines).rstrip() + "\n"


def tag_body(tag: str) -> str:
    return git("tag", "--list", "--format=%(contents:body)", tag) or ""


def publish(release: dict) -> int:
    """Tags HEAD with the notes and pushes the tag: the local way in."""
    if (git("status", "--porcelain") or "").strip():
        raise Fault("the working tree is dirty; commit or stash it first")
    branch = (git("symbolic-ref", "--quiet", "--short", "HEAD", check=False) or "").strip()
    if branch != "fork/main":
        raise Fault(f"releases come from fork/main, not {branch or 'a detached HEAD'}")
    git("fetch", "origin", "fork/main")
    head = (git("rev-parse", "HEAD") or "").strip()
    pushed = (git("rev-parse", "origin/fork/main") or "").strip()
    if head != pushed:
        raise Fault("fork/main is not what origin has; pull or push first")
    tag = release["tag"]
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as notes:
        notes.write(render(release))
        path = notes.name
    try:
        # The default cleanup deletes every line that starts with '#', which
        # would take the notes' markdown headings with it.
        git("tag", "-a", "--cleanup=whitespace", tag, "-F", path)
        try:
            git("push", "origin", f"refs/tags/{tag}")
        except Fault:
            # A local tag is a release to `releases()`: leaving one behind
            # after a failed push would make the next run read it as the last
            # release and refuse with "nothing to release".
            git("tag", "-d", tag)
            raise
    finally:
        Path(path).unlink(missing_ok=True)
    print(f"release.py: tagged and pushed {tag}", file=sys.stderr)
    return 0


def check(tag: str) -> None:
    """Refuses a tag whose name, base or place in the sequence is wrong."""
    match = FORK_TAG.match(tag)
    if not match:
        raise Fault(f"{tag} is not v<upstream>-hearthdeck.<counter> (e.g. v1.7.0-hearthdeck.1.5.0)")
    base = tuple(int(part) for part in match.group(1, 2, 3))
    counter = tuple(int(part or 0) for part in match.group(4, 5, 6))
    if not exists(f"refs/tags/{tag}"):
        raise Fault(f"no tag {tag}; fetch it first (git fetch --tags)")
    if (git("cat-file", "-t", f"refs/tags/{tag}") or "").strip() != "tag":
        raise Fault(f"{tag} is a lightweight tag; the notes live in an annotated tag's message")
    built = workspace_version(tag)
    if built != base:
        raise Fault(
            f"{tag} names upstream {join(base)}, but Cargo.toml at the tag says {join(built)}; "
            "the base comes from the tree, so the tag is wrong"
        )
    history = releases(rev=None)
    later = [release for release in history if release["base"] > base]
    if later:
        raise Fault(f"{tag} builds on {join(base)}, which is behind {later[-1]['tag']}")
    same = [release for release in history if release["base"] == base and release["tag"] != tag]
    if same and same[-1]["counter"] >= counter:
        raise Fault(f"{tag} does not follow {same[-1]['tag']}; its counter must be higher")
    if not tag_body(tag).strip():
        print(
            f"release.py: warning: {tag} carries no release notes; the release will only list the packages",
            file=sys.stderr,
        )


def preview(release: dict, text: str) -> int:
    if (git("status", "--porcelain") or "").strip():
        print("release.py: the working tree is dirty; a release comes from a commit", file=sys.stderr)
    branch = (git("symbolic-ref", "--quiet", "--short", "HEAD", check=False) or "").strip()
    if branch and branch != "fork/main":
        print(f"release.py: HEAD is on {branch}, not fork/main", file=sys.stderr)
    newer = newest_unmerged_upstream(release["base"])
    if newer:
        print(
            f"release.py: upstream released {newer}, which fork/main has not merged; "
            "merge it first if it belongs in this release",
            file=sys.stderr,
        )
    counter = join(release["counter"])
    print(
        f"release.py: {release['tag']} (fork counter {counter}, {release['bump']} bump)",
        file=sys.stderr,
    )
    print(text, end="")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Works out the fork's next release and its notes.",
        epilog="See docs/RELEASING.md for the whole flow.",
    )
    what = parser.add_mutually_exclusive_group()
    what.add_argument("--tag", action="store_true", help="print only the next tag name")
    what.add_argument("--notes", action="store_true", help="print only the release notes")
    what.add_argument("--publish", action="store_true", help="tag HEAD with the notes and push it")
    what.add_argument("--check", metavar="TAG", help="validate an existing tag and exit")
    parser.add_argument(
        "--bump",
        choices=("auto", "patch", "minor", "major"),
        default="auto",
        help="how the fork counter moves (default: auto, from the commits)",
    )
    args = parser.parse_args(argv)
    try:
        if args.check:
            check(args.check)
            return 0
        release = plan(args.bump)
        if args.tag:
            print(release["tag"])
            return 0
        if args.publish:
            return publish(release)
        text = render(release)
        if args.notes:
            print(text, end="")
            return 0
        return preview(release, text)
    except Fault as fault:
        print(f"release.py: {fault}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
