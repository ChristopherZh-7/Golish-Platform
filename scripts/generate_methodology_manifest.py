#!/usr/bin/env python3
"""Generate a deterministic Golish methodology corpus manifest.

This is an offline packaging helper. It never executes skill bodies; it reads
only SKILL.md frontmatter and hashes the complete source bytes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path

import yaml


PARSER_CONTRACT = "golish-methodology-skill-parser@1"
INDEX_CONTRACT = "golish-methodology-tag-index@1"
VALID_TAG = re.compile(r"^[a-z0-9_.:/-]{1,128}$")


def sha256_bytes(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def serde_json_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")


def frontmatter(path: Path) -> dict[str, object]:
    content = path.read_text(encoding="utf-8")
    if not content.startswith("---"):
        raise ValueError(f"{path}: missing YAML frontmatter")
    closing = content.find("\n---", 3)
    if closing < 0:
        raise ValueError(f"{path}: missing closing YAML frontmatter delimiter")
    parsed = yaml.safe_load(content[3:closing].strip())
    if not isinstance(parsed, dict):
        raise ValueError(f"{path}: frontmatter is not an object")
    name = parsed.get("name")
    description = parsed.get("description")
    if not isinstance(name, str) or not 1 <= len(name.encode("utf-8")) <= 256:
        raise ValueError(f"{path}: name is missing or outside 1..=256 bytes")
    if description is not None and (
        not isinstance(description, str)
        or not 1 <= len(description.encode("utf-8")) <= 4096
    ):
        raise ValueError(f"{path}: description is outside the data-only contract")
    return parsed


def canonical_skill_documents(root: Path) -> list[Path]:
    """Discover exact-case SKILL.md files without following symlinks.

    Using ``Path.glob('**/SKILL.md')`` is unsafe on case-insensitive file
    systems because it can also resolve an upstream ``skill.md`` as
    ``SKILL.md``. Directory enumeration preserves the actual entry name.
    """

    skills_root = root / "skills"
    documents: list[Path] = []

    def reject_walk_error(error: OSError) -> None:
        raise error

    for directory, directory_names, file_names in os.walk(
        skills_root,
        topdown=True,
        onerror=reject_walk_error,
        followlinks=False,
    ):
        directory_path = Path(directory)
        for name in directory_names:
            path = directory_path / name
            if path.is_symlink():
                raise ValueError(f"{path}: methodology directory must not be a symlink")
        for name in file_names:
            path = directory_path / name
            if path.is_symlink() or not path.is_file():
                raise ValueError(f"{path}: methodology source must be a regular file")
            if name == "SKILL.md":
                documents.append(path)
    return sorted(documents, key=lambda path: path.relative_to(root).as_posix())


def normalized_tags(metadata: dict[str, object]) -> list[str]:
    candidates: list[str] = []
    for key in (
        "tags",
        "tech_stack",
        "cwe_ids",
        "chains_with",
        "prerequisites",
        "platforms",
        "all_tactics",
    ):
        value = metadata.get(key)
        if isinstance(value, list):
            candidates.extend(item for item in value if isinstance(item, str))
    for key in ("name", "category", "technique_id", "tactic"):
        value = metadata.get(key)
        if isinstance(value, str):
            candidates.append(value)
    name = str(metadata["name"])
    candidates.extend(part for part in re.split(r"[-_]", name) if len(part) > 1)
    tags = {
        candidate.strip().lower()
        for candidate in candidates
        if VALID_TAG.fullmatch(candidate.strip().lower())
    }
    if not tags:
        raise ValueError(f"skill {name!r} yielded no contract-valid methodology tags")
    return sorted(tags)


def build_manifest(args: argparse.Namespace) -> dict[str, object]:
    root = args.root.resolve(strict=True)
    license_path = root / args.license_file
    documents: list[dict[str, object]] = []
    for path in canonical_skill_documents(root):
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"{path}: methodology document must be a regular file")
        relative_path = path.relative_to(root).as_posix()
        content_sha256 = sha256_bytes(path.read_bytes())
        document_id = "document:" + hashlib.sha256(
            serde_json_bytes([relative_path, content_sha256])
        ).hexdigest()
        documents.append(
            {
                "document_id": document_id,
                "relative_path": relative_path,
                "content_sha256": content_sha256,
                "tags": normalized_tags(frontmatter(path)),
            }
        )
    if not documents:
        raise ValueError(f"{root}: no skills/**/SKILL.md documents found")

    root_members = [
        {
            "document_id": item["document_id"],
            "relative_path": item["relative_path"],
            "content_sha256": item["content_sha256"],
            "normalized_tags": item["tags"],
        }
        for item in documents
    ]
    content_root_sha256 = sha256_bytes(serde_json_bytes(root_members))
    identity = {
        "source_kind": args.source_kind,
        "upstream_url": args.upstream_url,
        "upstream_revision": args.upstream_revision,
        "license_spdx": args.license_spdx,
        "license_text_sha256": sha256_bytes(license_path.read_bytes()),
        "document_count": len(documents),
        "content_root_sha256": content_root_sha256,
        "parser_contract_version": PARSER_CONTRACT,
        "index_contract_version": INDEX_CONTRACT,
    }
    return {
        "schema_version": "methodology_corpus_manifest.v1",
        "corpus_id": "corpus:" + hashlib.sha256(serde_json_bytes(identity)).hexdigest(),
        **identity,
        "signature_state": args.signature_state,
        "trust_store_epoch": args.trust_store_epoch,
        "ingested_at": args.ingested_at,
        "superseded_at": None,
        "documents": documents,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-kind", default="third_party_skill_corpus")
    parser.add_argument("--upstream-url", required=True)
    parser.add_argument("--upstream-revision", required=True)
    parser.add_argument("--license-spdx", required=True)
    parser.add_argument("--license-file", default="LICENSE")
    parser.add_argument("--signature-state", default="content_addressed")
    parser.add_argument("--trust-store-epoch", type=int, default=1)
    parser.add_argument("--ingested-at", default="2026-08-12T00:00:00Z")
    args = parser.parse_args()

    manifest = build_manifest(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_name(f".{args.output.name}.tmp")
    temporary.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, args.output)
    print(
        json.dumps(
            {
                "corpus_id": manifest["corpus_id"],
                "content_root_sha256": manifest["content_root_sha256"],
                "document_count": manifest["document_count"],
                "output": str(args.output),
            },
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main()
